use std::path::Path;

use crate::error::ResearchError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorpusClass {
    Synthetic,
    Live,
    LockedHoldout,
}

impl CorpusClass {
    #[must_use]
    pub fn from_path(path: &Path) -> Self {
        let mut live = false;
        let mut locked = false;
        for component in path.iter() {
            let raw = component.to_string_lossy();
            let lowered = raw.to_ascii_lowercase();
            let stem = stem_of(&lowered);
            if is_live_marker(&lowered, stem) {
                live = true;
            }
            if is_locked_marker(&lowered, stem) {
                locked = true;
            }
        }
        if live {
            Self::Live
        } else if locked {
            Self::LockedHoldout
        } else {
            Self::Synthetic
        }
    }

    pub fn admit_load(self) -> Result<(), ResearchError> {
        match self {
            Self::Synthetic => Ok(()),
            Self::Live => Err(ResearchError::LiveCorpusForbidden),
            Self::LockedHoldout => Err(ResearchError::LockedCorpusForbidden),
        }
    }
}

pub fn refuse_corpus_path(path: impl AsRef<Path>) -> Result<(), ResearchError> {
    CorpusClass::from_path(path.as_ref()).admit_load()
}

pub fn load_corpus_path(path: impl AsRef<Path>) -> Result<Vec<u8>, ResearchError> {
    let path = path.as_ref();
    refuse_corpus_path(path)?;
    std::fs::read(path).map_err(|_| ResearchError::InvalidFixture)
}

fn stem_of(name: &str) -> &str {
    name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name)
}

fn is_live_marker(component: &str, stem: &str) -> bool {
    matches_marker(component, stem, &["live", "live_corpus", "live-corpus"])
        || component == "replica_cmds"
        || stem == "replica_cmds"
}

fn is_locked_marker(component: &str, stem: &str) -> bool {
    component.ends_with(".lock")
        || matches_marker(
            component,
            stem,
            &[
                "locked-holdout",
                "locked_holdout",
                "locked-corpus",
                "locked_corpus",
            ],
        )
}

fn matches_marker(component: &str, stem: &str, markers: &[&str]) -> bool {
    markers
        .iter()
        .any(|marker| component == *marker || stem == *marker)
}
