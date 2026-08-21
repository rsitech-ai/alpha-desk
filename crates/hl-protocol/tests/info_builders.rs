use std::fs;
use std::path::{Path, PathBuf};

use domain_types::KnownTime;
use hl_protocol::info::{
    ArchiveRef, InfoParseContext, parse_approved_builders, parse_extra_agents,
};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/hyperliquid/official-info")
}

fn context() -> InfoParseContext {
    InfoParseContext::new(
        blake3::hash(b"t06-builders"),
        KnownTime::from_unix_micros(1_721_000_000_000_000).expect("time"),
        ArchiveRef::new("fixture:t06-builders").expect("archive ref"),
    )
}

fn read_fixture(name: &str) -> Vec<u8> {
    fs::read(fixture_root().join(name)).expect("fixture")
}

#[test]
fn info_extra_agents_and_approved_builders_do_not_steal_account_identity() {
    let agents = parse_extra_agents(&read_fixture("response-extra-agents.json"), context())
        .expect("agents")
        .1;
    assert_eq!(agents.agents()[0].name(), "Mobile QR");
    assert_eq!(
        agents.agents()[0].address().to_api_string(),
        "0x1715462edd45a87eea74e402428392ffc744eb20"
    );

    let builders =
        parse_approved_builders(&read_fixture("response-approved-builders.json"), context())
            .expect("builders")
            .1;
    assert_eq!(
        builders.builders()[0].to_api_string(),
        "0x476fa87b4d3818f437f38f1263bee508d7672d82"
    );
    assert_ne!(
        builders.builders()[0],
        agents.agents()[0].address(),
        "builder approval is not the agent wallet"
    );
}
