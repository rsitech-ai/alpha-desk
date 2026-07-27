use canonical_events::{EventPayload, TradeMatched};
use domain_types::Address;
use test_fixtures::TradeScenarioBuilder;

#[test]
fn matched_trade_is_deterministic_and_preserves_the_explicit_seed() {
    let buyer = Address::from_bytes([0x11; 20]);
    let seller = Address::from_bytes([0x22; 20]);

    let first = TradeScenarioBuilder::at_block(42)
        .with_seed(7)
        .matched_trade(buyer, seller)
        .unwrap();
    let second = TradeScenarioBuilder::at_block(42)
        .with_seed(7)
        .matched_trade(buyer, seller)
        .unwrap();

    assert_eq!(first, second);
    assert_eq!(first.block_time().unix_micros(), 1_721_779_200_000_042);
    assert_eq!(first.event_id().as_str(), "fixture-mainnet-42-0-0");
    assert!(matches!(
        first.payload(),
        EventPayload::TradeMatched(TradeMatched {
            deterministic_seed: 7,
            ..
        })
    ));
}

#[test]
fn matched_trade_defaults_to_seed_zero_and_exact_values() {
    let envelope = TradeScenarioBuilder::at_block(42)
        .matched_trade(
            Address::from_bytes([0x11; 20]),
            Address::from_bytes([0x22; 20]),
        )
        .unwrap();

    let EventPayload::TradeMatched(trade) = envelope.payload() else {
        panic!("the trade builder must produce a TradeMatched payload");
    };
    assert_eq!(trade.price.to_string(), "65000.000000");
    assert_eq!(trade.quantity.to_string(), "0.01000000");
    assert_eq!(trade.deterministic_seed, 0);
}

#[test]
fn block_height_overflow_is_returned_as_a_typed_error() {
    let error = TradeScenarioBuilder::at_block(u64::MAX)
        .matched_trade(
            Address::from_bytes([0x11; 20]),
            Address::from_bytes([0x22; 20]),
        )
        .unwrap_err();

    assert!(error.to_string().contains("fixture block time overflow"));
}
