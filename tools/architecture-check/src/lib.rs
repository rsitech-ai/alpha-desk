use cargo_metadata::{Metadata, MetadataCommand, PackageId};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fs::File;
use std::path::Path;

const FEATURE_RUNTIME_RULE: &str = "feature definitions must not depend on an inference runtime";
const DOMAIN_STORAGE_RULE: &str = "domain types must not depend on storage ports";
const DOMAIN_SERVICE_RULE: &str = "domain crates must not depend on service packages";
const HL_EXEC_RULE: &str = "hl-exec is excluded from every V1 dependency graph";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Layer {
    DomainCrate,
    Service,
    Tool,
    Other,
}

#[derive(Clone, Debug)]
struct PackageInfo {
    name: String,
    layer: Layer,
}

#[derive(Debug)]
struct Graph {
    packages: HashMap<PackageId, PackageInfo>,
    edges: HashMap<PackageId, Vec<PackageId>>,
    workspace: HashSet<PackageId>,
}

#[derive(Debug, Eq, PartialEq)]
struct Violation {
    path: Vec<PackageId>,
    rule: &'static str,
}

pub fn load_metadata(path: Option<&Path>) -> Result<Metadata, String> {
    if let Some(path) = path {
        let file =
            File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
        return serde_json::from_reader(file)
            .map_err(|error| format!("cannot parse {}: {error}", path.display()));
    }

    let mut command = MetadataCommand::new();
    command.other_options(vec![
        "--locked".to_owned(),
        "--offline".to_owned(),
        "--all-features".to_owned(),
    ]);
    command
        .exec()
        .map_err(|error| format!("cargo metadata failed: {error}"))
}

pub fn check(metadata: &Metadata) -> Result<Vec<String>, String> {
    let graph = Graph::from_metadata(metadata)?;
    let mut diagnostics = graph.policy_violations();

    if let Some(cycle) = graph.shortest_workspace_cycle() {
        diagnostics.push(format!("cyclic-dependency: {}", graph.render_path(&cycle)));
    }

    diagnostics.sort();
    Ok(diagnostics)
}

impl Graph {
    fn from_metadata(metadata: &Metadata) -> Result<Self, String> {
        let resolve = metadata
            .resolve
            .as_ref()
            .ok_or_else(|| "cargo metadata resolve graph is missing".to_owned())?;

        let mut packages = HashMap::new();
        for package in &metadata.packages {
            let layer = package
                .manifest_path
                .strip_prefix(&metadata.workspace_root)
                .ok()
                .map(|path| classify_layer(path.as_str()))
                .unwrap_or(Layer::Other);
            let info = PackageInfo {
                name: package.name.to_string(),
                layer,
            };
            if packages.insert(package.id.clone(), info).is_some() {
                return Err(format!("duplicate package id: {}", package.id));
            }
        }

        let mut workspace_ids = metadata.workspace_members.clone();
        workspace_ids.sort();
        let workspace: HashSet<_> = workspace_ids.iter().cloned().collect();
        for id in &workspace_ids {
            if !packages.contains_key(id) {
                return Err(format!("workspace member is absent from packages: {id}"));
            }
        }

        let mut edges = HashMap::new();
        let mut node_ids = BTreeSet::new();
        for node in &resolve.nodes {
            if !packages.contains_key(&node.id) {
                return Err(format!("resolve node is absent from packages: {}", node.id));
            }
            if !node_ids.insert(node.id.clone()) {
                return Err(format!("duplicate resolve node: {}", node.id));
            }

            let mut dependencies = Vec::with_capacity(node.deps.len());
            for dependency in &node.deps {
                if !packages.contains_key(&dependency.pkg) {
                    return Err(format!(
                        "resolved dependency is absent from packages: {} -> {}",
                        node.id, dependency.pkg
                    ));
                }
                dependencies.push(dependency.pkg.clone());
            }
            dependencies.sort_by(|left, right| {
                self_sort_key(&packages, left).cmp(&self_sort_key(&packages, right))
            });
            dependencies.dedup();
            edges.insert(node.id.clone(), dependencies);
        }

        for id in &workspace_ids {
            if !node_ids.contains(id) {
                return Err(format!(
                    "workspace member is absent from resolve nodes: {id}"
                ));
            }
        }

        Ok(Self {
            packages,
            edges,
            workspace,
        })
    }

    fn policy_violations(&self) -> Vec<String> {
        let mut violations = Vec::new();

        self.add_named_path_violation(
            &mut violations,
            "domain-types",
            "storage-ports",
            DOMAIN_STORAGE_RULE,
        );
        self.add_named_path_violation(
            &mut violations,
            "feature-core",
            "model-runtime",
            FEATURE_RUNTIME_RULE,
        );

        for source in self.sorted_workspace_ids() {
            if self.packages[&source].layer == Layer::DomainCrate
                && let Some(path) = self.shortest_path(&source, |id| {
                    id != &source && self.packages[id].layer == Layer::Service
                })
            {
                violations.push(Violation {
                    path,
                    rule: DOMAIN_SERVICE_RULE,
                });
            }

            if let Some(path) = self.shortest_path(&source, |id| {
                id != &source && self.packages[id].name == "hl-exec"
            }) {
                violations.push(Violation {
                    path,
                    rule: HL_EXEC_RULE,
                });
            }
        }

        violations.sort_by(|left, right| {
            self.violation_sort_key(left)
                .cmp(&self.violation_sort_key(right))
        });
        violations.dedup_by(|left, right| {
            left.rule == right.rule
                && self.render_identity_path(&left.path) == self.render_identity_path(&right.path)
        });
        violations
            .into_iter()
            .map(|violation| {
                format!(
                    "forbidden-dependency: {}\nrule: {}",
                    self.render_path(&violation.path),
                    violation.rule
                )
            })
            .collect()
    }

    fn add_named_path_violation(
        &self,
        violations: &mut Vec<Violation>,
        source_name: &str,
        target_name: &str,
        rule: &'static str,
    ) {
        for source in self
            .sorted_workspace_ids()
            .into_iter()
            .filter(|id| self.packages[id].name == source_name)
        {
            if let Some(path) =
                self.shortest_path(&source, |id| self.packages[id].name == target_name)
            {
                violations.push(Violation { path, rule });
            }
        }
    }

    fn shortest_path(
        &self,
        start: &PackageId,
        is_target: impl Fn(&PackageId) -> bool,
    ) -> Option<Vec<PackageId>> {
        let mut queue = VecDeque::from([start.clone()]);
        let mut previous: HashMap<PackageId, Option<PackageId>> =
            HashMap::from([(start.clone(), None)]);

        while let Some(current) = queue.pop_front() {
            if current != *start && is_target(&current) {
                return Some(reconstruct_path(&previous, current));
            }
            for dependency in self.edges.get(&current).into_iter().flatten() {
                if !previous.contains_key(dependency) {
                    previous.insert(dependency.clone(), Some(current.clone()));
                    queue.push_back(dependency.clone());
                }
            }
        }
        None
    }

    fn shortest_workspace_cycle(&self) -> Option<Vec<PackageId>> {
        let mut cycles = Vec::new();

        for start in self.sorted_workspace_ids() {
            for dependency in self.edges.get(&start).into_iter().flatten() {
                if !self.workspace.contains(dependency) {
                    continue;
                }
                if dependency == &start {
                    cycles.push(vec![start.clone(), start.clone()]);
                    continue;
                }
                if let Some(mut return_path) = self.shortest_workspace_path(dependency, &start) {
                    let mut cycle = vec![start.clone()];
                    cycle.append(&mut return_path);
                    cycles.push(cycle);
                }
            }
        }

        cycles.into_iter().min_by(|left, right| {
            left.len().cmp(&right.len()).then_with(|| {
                self.render_identity_path(left)
                    .cmp(&self.render_identity_path(right))
            })
        })
    }

    fn shortest_workspace_path(
        &self,
        start: &PackageId,
        target: &PackageId,
    ) -> Option<Vec<PackageId>> {
        let mut queue = VecDeque::from([start.clone()]);
        let mut previous: HashMap<PackageId, Option<PackageId>> =
            HashMap::from([(start.clone(), None)]);

        while let Some(current) = queue.pop_front() {
            if &current == target {
                return Some(reconstruct_path(&previous, current));
            }
            for dependency in self.edges.get(&current).into_iter().flatten() {
                if self.workspace.contains(dependency) && !previous.contains_key(dependency) {
                    previous.insert(dependency.clone(), Some(current.clone()));
                    queue.push_back(dependency.clone());
                }
            }
        }
        None
    }

    fn sorted_workspace_ids(&self) -> Vec<PackageId> {
        let mut ids: Vec<_> = self.workspace.iter().cloned().collect();
        ids.sort_by_key(|id| self.package_sort_key(id));
        ids
    }

    fn package_sort_key(&self, id: &PackageId) -> (String, String) {
        self_sort_key(&self.packages, id)
    }

    fn violation_sort_key(&self, violation: &Violation) -> (usize, String, &'static str) {
        (
            violation.path.len(),
            self.render_identity_path(&violation.path),
            violation.rule,
        )
    }

    fn render_path(&self, path: &[PackageId]) -> String {
        path.iter()
            .map(|id| self.packages[id].name.as_str())
            .collect::<Vec<_>>()
            .join(" -> ")
    }

    fn render_identity_path(&self, path: &[PackageId]) -> String {
        path.iter()
            .map(|id| format!("{} ({id})", self.packages[id].name))
            .collect::<Vec<_>>()
            .join(" -> ")
    }
}

fn classify_layer(relative_manifest: &str) -> Layer {
    if relative_manifest.starts_with("crates/") {
        Layer::DomainCrate
    } else if relative_manifest.starts_with("services/") {
        Layer::Service
    } else if relative_manifest.starts_with("tools/") {
        Layer::Tool
    } else {
        Layer::Other
    }
}

fn self_sort_key(packages: &HashMap<PackageId, PackageInfo>, id: &PackageId) -> (String, String) {
    (
        packages
            .get(id)
            .map(|package| package.name.clone())
            .unwrap_or_default(),
        id.repr.clone(),
    )
}

fn reconstruct_path(
    previous: &HashMap<PackageId, Option<PackageId>>,
    mut current: PackageId,
) -> Vec<PackageId> {
    let mut path = vec![current.clone()];
    while let Some(Some(parent)) = previous.get(&current) {
        path.push(parent.clone());
        current = parent.clone();
    }
    path.reverse();
    path
}
