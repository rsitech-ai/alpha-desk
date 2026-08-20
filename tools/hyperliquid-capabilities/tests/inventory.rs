use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use hyperliquid_capabilities::{
    parse_manifest, parse_request_cost_base_weight, rest_info_base_weight, validate_manifest,
    SourceRole, StateTarget, REST_INFO_WEIGHT_2,
};

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn committed_manifest() -> hyperliquid_capabilities::Manifest {
    let root = workspace_root();
    let source = fs::read_to_string(root.join("config/hyperliquid/capabilities.toml"))
        .expect("committed manifest");
    let manifest = parse_manifest(&source).expect("committed manifest must parse");
    validate_manifest(&manifest).expect("committed manifest must validate");
    manifest
}

fn info_general() -> &'static [&'static str] {
    &[
        "allMids",
        "openOrders",
        "frontendOpenOrders",
        "userFills",
        "userFillsByTime",
        "recentTrades",
        "userRateLimit",
        "orderStatus",
        "l2Book",
        "candleSnapshot",
        "exchangeStatus",
        "historicalOrders",
        "userTwapSliceFills",
        "userTwapSliceFillsByTime",
        "twapHistory",
        "subAccounts",
        "userToMultiSigSigners",
        "portfolio",
        "referral",
        "userFees",
        "userRole",
        "userAbstraction",
        "userDexAbstraction",
        "extraAgents",
        "approvedBuilders",
        "userVaultEquities",
        "vaultDetails",
        "delegatorSummary",
        "delegations",
        "delegatorHistory",
        "delegatorRewards",
        "validatorStats",
        "alignedQuoteTokenInfo",
        "borrowLendUserState",
        "borrowLendReserveState",
        "allBorrowLendReserveStates",
    ]
}

fn info_perp() -> &'static [&'static str] {
    &[
        "perpDexs",
        "meta",
        "metaAndAssetCtxs",
        "allPerpMetas",
        "clearinghouseState",
        "userFunding",
        "userNonFundingLedgerUpdates",
        "nonUserFundingUpdates",
        "fundingHistory",
        "predictedFundings",
        "perpsAtOpenInterestCap",
        "perpDeployAuctionStatus",
        "activeAssetData",
        "perpDexLimits",
        "perpDexStatus",
        "perpAnnotation",
        "perpCategories",
        "perpConciseAnnotations",
    ]
}

fn info_spot() -> &'static [&'static str] {
    &[
        "spotMeta",
        "spotMetaAndAssetCtxs",
        "spotClearinghouseState",
        "spotDeployState",
        "spotPairDeployAuctionStatus",
        "tokenDetails",
        "outcomeMeta",
    ]
}

fn websocket() -> &'static [&'static str] {
    &[
        "allMids",
        "notification",
        "webData3",
        "twapStates",
        "clearinghouseState",
        "openOrders",
        "candle",
        "l2Book",
        "trades",
        "orderUpdates",
        "userEvents",
        "userFills",
        "userFundings",
        "userNonFundingLedgerUpdates",
        "activeAssetCtx",
        "activeAssetData",
        "userTwapSliceFills",
        "userTwapHistory",
        "bbo",
        "spotState",
        "allDexsClearinghouseState",
        "allDexsAssetCtxs",
    ]
}

fn node_datasets() -> &'static [&'static str] {
    &[
        "transaction-blocks",
        "trades",
        "fills",
        "order-statuses",
        "raw-book-diffs",
        "misc-events",
        "market-metadata",
        "abci-state-snapshots",
        "l4-snapshots",
    ]
}

fn s3_datasets() -> &'static [&'static str] {
    &[
        "l2-snapshots",
        "asset-contexts",
        "node_fills_by_block",
        "node-fills-legacy",
        "node-trades-legacy",
        "explorer-blocks",
        "replica_cmds",
        "hyperevm-blocks",
        "hyperevm-receipts",
    ]
}

fn expected_set(values: &[&'static str]) -> BTreeSet<&'static str> {
    values.iter().copied().collect()
}

fn identifiers_for<'a>(
    capabilities: &'a [hyperliquid_capabilities::Capability],
    transport: &str,
) -> BTreeSet<&'a str> {
    capabilities
        .iter()
        .filter(|capability| capability.transport == transport)
        .map(|capability| capability.identifier.as_str())
        .collect()
}

fn transport_count(
    capabilities: &[hyperliquid_capabilities::Capability],
    transport: &str,
) -> usize {
    capabilities
        .iter()
        .filter(|capability| capability.transport == transport)
        .count()
}

#[test]
fn committed_manifest_covers_appendix_b_and_current_datasets() {
    let root = workspace_root();
    let manifest = committed_manifest();

    assert_eq!(info_general().len(), 36, "Appendix B shared/general /info");
    assert_eq!(info_perp().len(), 18, "Appendix B perpetual /info");
    assert_eq!(info_spot().len(), 7, "Appendix B spot/outcome /info");
    assert_eq!(websocket().len(), 22, "Appendix B WebSocket");
    assert_eq!(node_datasets().len(), 9, "spec §13.1 node datasets");
    assert_eq!(s3_datasets().len(), 9, "spec §13.3 S3 datasets");

    let expected_info: BTreeSet<&str> = info_general()
        .iter()
        .chain(info_perp())
        .chain(info_spot())
        .copied()
        .collect();
    assert_eq!(expected_info.len(), 61);
    assert_eq!(transport_count(&manifest.capability, "rest_info"), 61);
    assert_eq!(
        identifiers_for(&manifest.capability, "rest_info"),
        expected_info
    );

    assert_eq!(transport_count(&manifest.capability, "websocket"), 22);
    assert_eq!(
        identifiers_for(&manifest.capability, "websocket"),
        expected_set(websocket())
    );

    assert_eq!(transport_count(&manifest.capability, "node_files"), 9);
    assert_eq!(
        identifiers_for(&manifest.capability, "node_files"),
        expected_set(node_datasets())
    );

    assert_eq!(transport_count(&manifest.capability, "s3"), 9);
    assert_eq!(
        identifiers_for(&manifest.capability, "s3"),
        expected_set(s3_datasets())
    );

    assert_eq!(manifest.capability.len(), 101);

    let schema =
        fs::read_to_string(root.join("schemas/hyperliquid/capability-manifest-v1.schema.json"))
            .expect("schema");
    assert!(
        schema.contains("\"title\": \"Hyperliquid capability manifest\""),
        "schema must name the manifest"
    );
}

#[test]
fn rest_info_request_cost_follows_spec_12_1() {
    assert_eq!(REST_INFO_WEIGHT_2.len(), 6);
    for identifier in REST_INFO_WEIGHT_2 {
        assert_eq!(rest_info_base_weight(identifier), 2, "{identifier}");
    }
    assert_eq!(rest_info_base_weight("userRole"), 60);

    let manifest = committed_manifest();
    let mut rest_info = 0;
    for capability in &manifest.capability {
        if capability.transport != "rest_info" {
            continue;
        }
        rest_info += 1;
        let expected = rest_info_base_weight(&capability.identifier);
        let actual =
            parse_request_cost_base_weight(&capability.request_cost).unwrap_or_else(|| {
                panic!(
                    "{} request_cost {} is not a base weight",
                    capability.id, capability.request_cost
                )
            });
        assert_eq!(
            actual, expected,
            "{} identifier {} cost {}",
            capability.id, capability.identifier, capability.request_cost
        );
        if expected == 20 {
            assert!(
                !REST_INFO_WEIGHT_2.contains(&capability.identifier.as_str())
                    && capability.identifier != "userRole"
            );
        }
    }
    assert_eq!(rest_info, 61);
}

#[test]
fn node_streams_and_replica_cmds_are_committed() {
    let manifest = committed_manifest();
    let node_files: Vec<_> = manifest
        .capability
        .iter()
        .filter(|capability| capability.transport == "node_files")
        .collect();
    assert_eq!(node_files.len(), 9);
    for capability in &node_files {
        assert_eq!(
            capability.source_role,
            SourceRole::Committed,
            "{}",
            capability.id
        );
    }
    let replica = manifest
        .capability
        .iter()
        .find(|row| row.id == "s3.replica_cmds")
        .expect("s3.replica_cmds");
    assert_eq!(replica.source_role, SourceRole::Committed);
}

#[test]
fn hyperevm_s3_rows_are_evm_fact() {
    let manifest = committed_manifest();
    for id in ["s3.hyperevm_blocks", "s3.hyperevm_receipts"] {
        let capability = manifest
            .capability
            .iter()
            .find(|row| row.id == *id)
            .unwrap_or_else(|| panic!("missing {id}"));
        assert_eq!(capability.state_target, StateTarget::EvmFact, "{id}");
        assert!(
            !capability.state_target.is_state_affecting(),
            "{id} must not be HyperCore state-affecting"
        );
    }
}

#[test]
fn periodic_node_snapshots_omit_freshness_target() {
    let manifest = committed_manifest();
    for id in ["node.abci_state_snapshots", "node.l4_snapshots"] {
        let capability = manifest
            .capability
            .iter()
            .find(|row| row.id == *id)
            .unwrap_or_else(|| panic!("missing {id}"));
        assert_eq!(capability.freshness_target_ms, None, "{id}");
    }
}

#[test]
fn schema_lists_section_4_1_mapping_targets() {
    let schema = fs::read_to_string(
        workspace_root().join("schemas/hyperliquid/capability-manifest-v1.schema.json"),
    )
    .expect("schema");
    for name in [
        "canonical_event",
        "reconciled_snapshot",
        "reference_snapshot",
        "evm_fact",
        "discovery_only",
    ] {
        assert!(
            schema.contains(&format!("\"{name}\"")),
            "schema must list {name}"
        );
    }
}

#[test]
fn remaining_s3_archives_are_evidence_only() {
    let manifest = committed_manifest();
    let archives: Vec<_> = manifest
        .capability
        .iter()
        .filter(|capability| capability.transport == "s3" && capability.id != "s3.replica_cmds")
        .collect();
    assert_eq!(archives.len(), 8);
    for capability in archives {
        assert_eq!(
            capability.source_role,
            SourceRole::EvidenceOnly,
            "{}",
            capability.id
        );
    }
}

#[test]
fn official_websocket_rows_are_provisional() {
    let manifest = committed_manifest();
    let rows: Vec<_> = manifest
        .capability
        .iter()
        .filter(|capability| capability.transport == "websocket")
        .collect();
    assert_eq!(rows.len(), 22);
    for capability in rows {
        assert_eq!(
            capability.source_role,
            SourceRole::Provisional,
            "{}",
            capability.id
        );
    }
}

#[test]
fn implemented_node_rows_point_at_the_node_v1_fixture_set() {
    let root = workspace_root();
    let manifest = committed_manifest();
    let implemented_node: Vec<_> = manifest
        .capability
        .iter()
        .filter(|capability| {
            capability.transport == "node_files" && capability.status.requires_fixture_set()
        })
        .collect();
    assert!(
        !implemented_node.is_empty(),
        "replica_cmds / NodeLine rows with parsers must be implemented*"
    );
    for capability in implemented_node {
        assert_eq!(capability.fixture_set, "node_v1");
        assert_eq!(
            capability.parser,
            "hl_protocol::node::v1::parse_node_record"
        );
        assert!(
            root.join("fixtures/source/node-v1").is_dir(),
            "node_v1 fixture set must exist"
        );
    }
}
