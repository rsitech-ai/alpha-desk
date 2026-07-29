use canonical_events::{EventIdentityInput, EventKind, compute_event_id};
use domain_types::{BlockHeight, ChainId, EventId, TransactionId};

fn event_id(
    chain_id: &str,
    block_height: u64,
    transaction_identity: &str,
    canonical_event_index: u32,
    event_kind: EventKind,
    schema_major: u64,
) -> EventId {
    let chain_id = ChainId::new(chain_id).expect("valid chain");
    let transaction_identity = TransactionId::new(transaction_identity).expect("valid transaction");
    compute_event_id(&EventIdentityInput {
        chain_id: &chain_id,
        block_height: BlockHeight::new(block_height),
        transaction_identity: &transaction_identity,
        canonical_event_index,
        event_kind,
        schema_major,
    })
}

#[test]
fn event_identity_is_deterministic_and_has_the_versioned_public_shape() {
    let first = event_id("mainnet", 42, "tx-7", 0, EventKind::TradeMatched, 1);
    let repeated = event_id("mainnet", 42, "tx-7", 0, EventKind::TradeMatched, 1);

    assert_eq!(first, repeated);
    assert_eq!(
        first.as_str(),
        "evt_80387df37a3389902e817f28474bc4e48029a85ad44d5d9a670f30a8247a5ab1"
    );
    assert!(first.as_str().starts_with("evt_"));
    assert_eq!(first.as_str().len(), 68);
    assert!(
        first.as_str()[4..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
}

#[test]
fn every_canonical_identity_field_changes_the_event_id() {
    let baseline = event_id("mainnet", 42, "tx-7", 0, EventKind::TradeMatched, 1);
    let mutations = [
        event_id("testnet", 42, "tx-7", 0, EventKind::TradeMatched, 1),
        event_id("mainnet", 43, "tx-7", 0, EventKind::TradeMatched, 1),
        event_id("mainnet", 42, "tx-8", 0, EventKind::TradeMatched, 1),
        event_id("mainnet", 42, "tx-7", 1, EventKind::TradeMatched, 1),
        event_id("mainnet", 42, "tx-7", 0, EventKind::OrderFilled, 1),
        event_id("mainnet", 42, "tx-7", 0, EventKind::TradeMatched, 2),
    ];

    for mutation in mutations {
        assert_ne!(baseline, mutation);
    }
}

#[test]
fn length_framing_prevents_variable_field_concatenation_ambiguity() {
    assert_ne!(
        event_id("ab", 42, "c", 0, EventKind::TradeMatched, 1),
        event_id("a", 42, "bc", 0, EventKind::TradeMatched, 1),
    );
}
