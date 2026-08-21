use std::fs;
use std::path::{Path, PathBuf};

use domain_types::KnownTime;
use hl_protocol::info::{ArchiveRef, InfoParseContext, parse_referral, parse_user_fees};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/hyperliquid/official-info")
}

fn context() -> InfoParseContext {
    InfoParseContext::new(
        blake3::hash(b"t06-fees"),
        KnownTime::from_unix_micros(1_721_000_000_000_000).expect("time"),
        ArchiveRef::new("fixture:t06-fees").expect("archive ref"),
    )
}

fn read_fixture(name: &str) -> Vec<u8> {
    fs::read(fixture_root().join(name)).expect("fixture")
}

#[test]
fn info_user_fees_and_referral_parse() {
    let fees = parse_user_fees(&read_fixture("response-user-fees.json"), context())
        .expect("fees")
        .1;
    assert_eq!(fees.daily_user_vlm()[0].date(), "2025-05-23");
    assert_eq!(
        fees.staking_link_user().expect("link").to_api_string(),
        "0x54c049d9c7d3c92c2462bf3d28e083f3d6805061"
    );

    let referral = parse_referral(&read_fixture("response-referral.json"), context())
        .expect("referral")
        .1;
    assert_eq!(
        referral
            .referred_by()
            .expect("referred")
            .referrer()
            .to_api_string(),
        "0x5ac99df645f3414876c816caa18b2d234024b487"
    );
    assert_eq!(referral.referrer_code(), Some("TEST"));
    assert_eq!(referral.referral_states().len(), 1);
}
