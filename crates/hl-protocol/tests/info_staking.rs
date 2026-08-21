use domain_types::KnownTime;
use hl_protocol::info::{
    ArchiveRef, InfoObservationKind, InfoParseContext, parse_delegations, parse_delegator_history,
    parse_delegator_rewards, parse_delegator_summary, parse_validator_stats,
};
use serde_json::json;

fn context() -> InfoParseContext {
    InfoParseContext::new(
        blake3::hash(b"t07-staking"),
        KnownTime::from_unix_micros(1_721_000_000_000_000).expect("time"),
        ArchiveRef::new("fixture:t07-staking").expect("archive ref"),
    )
}

#[test]
fn info_staking_family_parses() {
    let summary = json!({
        "delegated": "12060.16529862",
        "undelegated": "0.0",
        "totalPendingWithdrawal": "0.0",
        "nPendingWithdrawals": 0
    });
    assert_eq!(
        parse_delegator_summary(&serde_json::to_vec(&summary).expect("json"), context())
            .expect("summary")
            .1
            .n_pending_withdrawals(),
        0
    );

    let delegations = json!([{
        "validator": "0x5ac99df645f3414876c816caa18b2d234024b487",
        "amount": "12060.16529862",
        "lockedUntilTimestamp": 1735466781353_i64
    }]);
    assert_eq!(
        parse_delegations(&serde_json::to_vec(&delegations).expect("json"), context())
            .expect("dels")
            .1
            .delegations()
            .len(),
        1
    );

    let history = json!([{
        "time": 1735380381353_i64,
        "hash": "0x55492465cb523f90815a041a226ba90147008d4b221a24ae8dc35a0dbede4ea4",
        "delta": {
            "delegate": {
                "validator": "0x5ac99df645f3414876c816caa18b2d234024b487",
                "amount": "10000.0",
                "isUndelegate": false
            }
        }
    }]);
    let parsed = parse_delegator_history(&serde_json::to_vec(&history).expect("json"), context())
        .expect("hist")
        .1;
    assert_eq!(parsed.kind(), InfoObservationKind::BoundedHistory);
    assert_eq!(parsed.updates()[0].delta_key(), "delegate");
    assert_eq!(parsed.updates()[0].is_undelegate(), Some(false));

    let rewards = json!([
        {"time": 1736726400073_i64, "source": "delegation", "totalAmount": "0.73117184"},
        {"time": 1736726400073_i64, "source": "commission", "totalAmount": "130.76445876"}
    ]);
    assert_eq!(
        parse_delegator_rewards(&serde_json::to_vec(&rewards).expect("json"), context())
            .expect("rewards")
            .1
            .rewards()[1]
            .source(),
        "commission"
    );

    let stats = json!([{
        "validator": "0x5ac99df645f3414876c816caa18b2d234024b487",
        "signer": "0x5ac99df645f3414876c816caa18b2d234024b487",
        "name": "test",
        "description": "d",
        "nRecentBlocks": 10,
        "stake": 1000,
        "isJailed": false,
        "unjailableAfter": null,
        "isActive": true,
        "commission": "0.05",
        "stats": [
            ["day", {"uptimeFraction": "0.99", "predictedApr": "0.1", "nSamples": 24}],
            ["week", {"uptimeFraction": "0.98", "predictedApr": "0.1", "nSamples": 168}],
            ["month", {"uptimeFraction": "0.97", "predictedApr": "0.1", "nSamples": 720}]
        ]
    }]);
    let validators = parse_validator_stats(&serde_json::to_vec(&stats).expect("json"), context())
        .expect("stats")
        .1;
    assert!(validators.validators()[0].is_active());
    assert_eq!(validators.validators()[0].stats()[0].period(), "day");
}
