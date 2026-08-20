use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use hyperliquid_capabilities::{parse_manifest, validate_manifest};

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
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

#[test]
fn committed_manifest_covers_appendix_b_and_current_datasets() {
    let root = workspace_root();
    let path = root.join("config/hyperliquid/capabilities.toml");
    let source = fs::read_to_string(&path).expect("committed manifest");
    let manifest = parse_manifest(&source).expect("committed manifest must parse");
    validate_manifest(&manifest).expect("committed manifest must validate");

    let info = identifiers_for(&manifest.capability, "rest_info");
    for identifier in info_general().iter().chain(info_perp()).chain(info_spot()) {
        assert!(
            info.contains(identifier),
            "missing /info family {identifier}"
        );
    }

    let ws = identifiers_for(&manifest.capability, "websocket");
    for identifier in websocket() {
        assert!(ws.contains(identifier), "missing WS family {identifier}");
    }

    let node = identifiers_for(&manifest.capability, "node_files");
    for identifier in node_datasets() {
        assert!(
            node.contains(identifier),
            "missing node dataset {identifier}"
        );
    }

    let s3 = identifiers_for(&manifest.capability, "s3");
    for identifier in s3_datasets() {
        assert!(s3.contains(identifier), "missing S3 dataset {identifier}");
    }

    let schema =
        fs::read_to_string(root.join("schemas/hyperliquid/capability-manifest-v1.schema.json"))
            .expect("schema");
    assert!(
        schema.contains("\"title\": \"Hyperliquid capability manifest\""),
        "schema must name the manifest"
    );
}

#[test]
fn implemented_node_rows_point_at_the_node_v1_fixture_set() {
    let root = workspace_root();
    let source = fs::read_to_string(root.join("config/hyperliquid/capabilities.toml"))
        .expect("committed manifest");
    let manifest = parse_manifest(&source).expect("parse");
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
