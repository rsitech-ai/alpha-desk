use std::fs;
use std::path::{Path, PathBuf};

use domain_types::KnownTime;
use hl_protocol::info::{
    ArchiveRef, InfoObservationKind, InfoParseContext, parse_spot_clearinghouse_state,
    parse_spot_deploy_state, parse_spot_meta, parse_spot_meta_and_asset_ctxs,
    parse_spot_pair_deploy_auction_status, parse_token_details,
};
use serde_json::json;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/hyperliquid/official-info")
}

fn context() -> InfoParseContext {
    InfoParseContext::new(
        blake3::hash(b"t07-spot"),
        KnownTime::from_unix_micros(1_721_000_000_000_000).expect("time"),
        ArchiveRef::new("fixture:t07-spot").expect("archive ref"),
    )
}

fn read_fixture(name: &str) -> Vec<u8> {
    fs::read(fixture_root().join(name)).expect("fixture")
}

#[test]
fn info_spot_id_versus_token_id() {
    let meta = parse_spot_meta(&read_fixture("response-spot-meta.json"), context())
        .expect("spot meta")
        .1;
    let pair = meta
        .universe()
        .iter()
        .find(|pair| pair.name() == "@107")
        .expect("@107");
    assert_eq!(pair.index(), 107);
    assert_eq!(pair.market_id().as_str(), "spot:@107");
    assert_eq!(pair.tokens(), &[150, 0]);

    let token = meta
        .tokens()
        .iter()
        .find(|token| token.index() == 150)
        .expect("hype token");
    assert_ne!(token.index(), pair.index());
    assert_eq!(
        token.token_id().as_str(),
        "0x0d01dc56dcaaca66ad901c959b4011ec"
    );
    assert_eq!(token.sz_decimals(), 2);
    assert_eq!(token.wei_decimals(), 8);
}

#[test]
fn info_spot_contexts_balances_deploy_and_token_details() {
    let ctxs = json!([
        {
            "tokens": [{
                "name": "USDC",
                "szDecimals": 8,
                "weiDecimals": 8,
                "index": 0,
                "tokenId": "0x6d1e7cde53ba9467b783cb7c530ce054",
                "isCanonical": true,
                "evmContract": null,
                "fullName": null
            }],
            "universe": [{
                "name": "PURR/USDC",
                "tokens": [1, 0],
                "index": 0,
                "isCanonical": true
            }]
        },
        [{"dayNtlVlm": "8906.0", "markPx": "0.14", "midPx": "0.209265", "prevDayPx": "0.20432"}]
    ]);
    let parsed =
        parse_spot_meta_and_asset_ctxs(&serde_json::to_vec(&ctxs).expect("json"), context())
            .expect("ctx")
            .1;
    assert_eq!(
        parsed.meta().universe()[0].market_id().as_str(),
        "spot:PURR/USDC"
    );

    let balances = json!({
        "balances": [{
            "coin": "USDC",
            "token": 0,
            "total": "14.625485",
            "hold": "0.0",
            "entryNtl": "0.0"
        }]
    });
    assert_eq!(
        parse_spot_clearinghouse_state(&serde_json::to_vec(&balances).expect("json"), context())
            .expect("spot ch")
            .1
            .kind(),
        InfoObservationKind::ReconciledSnapshot
    );

    let deploy = json!({
        "states": [{
            "token": 150,
            "spec": {"name": "HYPE", "szDecimals": 2, "weiDecimals": 8},
            "fullName": "Hyperliquid",
            "spots": [107],
            "maxSupply": "1000000000"
        }],
        "gasAuction": {
            "startTimeSeconds": 1733929200,
            "durationSeconds": 111600,
            "startGas": "181305.90046",
            "currentGas": null,
            "endGas": "181291.247358"
        }
    });
    let state = parse_spot_deploy_state(&serde_json::to_vec(&deploy).expect("json"), context())
        .expect("deploy")
        .1;
    assert_eq!(state.states()[0].token(), 150);
    assert_eq!(state.states()[0].spots(), &[107]);
    assert!(state.gas_auction().current_gas().is_none());

    let pair_auction = json!({
        "startTimeSeconds": 1755468000,
        "durationSeconds": 111600,
        "startGas": "500.0",
        "currentGas": "500.0",
        "endGas": null
    });
    assert!(
        parse_spot_pair_deploy_auction_status(
            &serde_json::to_vec(&pair_auction).expect("json"),
            context()
        )
        .expect("pair auction")
        .1
        .auction()
        .end_gas()
        .is_none()
    );

    let details = json!({
        "name": "TEST",
        "maxSupply": "1852229076.12716007",
        "totalSupply": "851681534.05516005",
        "circulatingSupply": "851681534.05516005",
        "szDecimals": 0,
        "weiDecimals": 5,
        "midPx": "3.2049",
        "markPx": "3.2025",
        "prevDayPx": "3.2025",
        "genesis": {"userBalances": [], "existingTokenBalances": []},
        "deployer": "0x0000000000000000000000000000000000000001",
        "deployGas": "100.0",
        "deployTime": "2024-06-05T10:50:59.434",
        "seededUsdc": "0.0",
        "nonCirculatingUserBalances": [],
        "futureEmissions": "0.0"
    });
    assert_eq!(
        parse_token_details(&serde_json::to_vec(&details).expect("json"), context())
            .expect("token")
            .1
            .kind(),
        InfoObservationKind::DirectLookup
    );
}
