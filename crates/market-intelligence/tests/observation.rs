use domain_types::{BlockHeight, FeatureSetVersion, Horizon, KnownTime, MarketId, ProtocolTime};
use feature_core::{FeatureValue, HealthAssessment, HealthState, MissingReason};
use market_intelligence::{
    BooleanObservationPurpose, MarketError, MarketFeatureSnapshot, ObservationMintKind,
    ObservationStatus, boolean_presence_from_decimal_depth, decimal_depth_from_boolean_presence,
    market_feature_key, mint_boolean_observation,
};

fn health() -> HealthAssessment {
    HealthAssessment::try_new("book", HealthState::Green, "synthetic").unwrap()
}

fn depth() -> FeatureValue {
    FeatureValue::Decimal {
        raw: 20_000 * 100_000_000,
        scale: 8,
    }
}

fn snapshot(
    book: FeatureValue,
    fills: FeatureValue,
    inventory: FeatureValue,
) -> MarketFeatureSnapshot {
    let mut values = std::collections::BTreeMap::new();
    values.insert(
        market_feature_key("registry").unwrap(),
        FeatureValue::Boolean(true),
    );
    values.insert(market_feature_key("book").unwrap(), book);
    values.insert(market_feature_key("fills").unwrap(), fills);
    values.insert(market_feature_key("inventory").unwrap(), inventory);
    MarketFeatureSnapshot::try_new(
        MarketId::new("BTC").unwrap(),
        Horizon::MINUTES_5,
        FeatureSetVersion::new("market-v1").unwrap(),
        ProtocolTime::from_unix_micros(1_000_000).unwrap(),
        KnownTime::from_unix_micros(1_000_000).unwrap(),
        BlockHeight::new(1),
        values,
        health(),
    )
    .unwrap()
}

#[test]
fn missing_decimal_depth_withholds_boolean_and_does_not_serialize_false() {
    let missing = FeatureValue::Missing(MissingReason::NotObserved);
    let converted = boolean_presence_from_decimal_depth(Some(&missing)).unwrap();
    assert_eq!(converted, FeatureValue::Missing(MissingReason::NotObserved));
    assert_ne!(converted, FeatureValue::Boolean(false));
    assert_eq!(
        boolean_presence_from_decimal_depth(None).unwrap(),
        FeatureValue::Missing(MissingReason::NotObserved)
    );
    assert_eq!(
        mint_boolean_observation(Some(&missing), BooleanObservationPurpose::Presence).unwrap(),
        FeatureValue::Missing(MissingReason::NotObserved)
    );
}

#[test]
fn observed_decimal_depth_mints_presence_true_not_live_or_ready() {
    let converted = boolean_presence_from_decimal_depth(Some(&depth())).unwrap();
    assert_eq!(converted, FeatureValue::Boolean(true));
    for purpose in [
        BooleanObservationPurpose::Crossed,
        BooleanObservationPurpose::Live,
        BooleanObservationPurpose::Ready,
    ] {
        assert!(matches!(
            mint_boolean_observation(Some(&depth()), purpose),
            Err(MarketError::Malformed {
                reason: "decimal depth cannot mint live, ready, or crossed",
                ..
            })
        ));
    }
}

#[test]
fn boolean_cannot_mint_decimal_depth() {
    let present = FeatureValue::Boolean(true);
    assert!(matches!(
        decimal_depth_from_boolean_presence(Some(&present)),
        Err(MarketError::Malformed {
            what: "observation",
            reason: "boolean cannot mint decimal depth",
        })
    ));
    assert_eq!(
        decimal_depth_from_boolean_presence(Some(&FeatureValue::Missing(
            MissingReason::NotObserved
        )))
        .unwrap(),
        FeatureValue::Missing(MissingReason::NotObserved)
    );
}

#[test]
fn typed_mint_refuses_cross_kind_without_conversion() {
    assert!(matches!(
        ObservationStatus::from_typed_feature(
            Some(&FeatureValue::Boolean(true)),
            ObservationMintKind::DecimalDepth
        ),
        Err(MarketError::Malformed {
            what: "observation",
            reason: "boolean cannot mint decimal depth",
        })
    ));
    assert!(matches!(
        ObservationStatus::from_typed_feature(Some(&depth()), ObservationMintKind::BooleanPresence),
        Err(MarketError::Malformed {
            what: "observation",
            reason: "decimal depth cannot mint boolean presence",
        })
    ));
    assert_eq!(
        ObservationStatus::from_typed_feature(
            Some(&FeatureValue::Boolean(false)),
            ObservationMintKind::BooleanPresence
        )
        .unwrap(),
        ObservationStatus::Missing(MissingReason::NotObserved)
    );
}

#[test]
fn boolean_book_cannot_mint_observed_proof() {
    let boolean_book = snapshot(
        FeatureValue::Boolean(true),
        FeatureValue::Boolean(true),
        depth(),
    );
    assert!(matches!(
        boolean_book.require_observed_book_and_fills(),
        Err(MarketError::Malformed {
            what: "book",
            reason: "boolean cannot mint decimal depth",
        })
    ));
}

#[test]
fn boolean_inventory_cannot_mint_observed_proof() {
    let boolean_inventory = snapshot(
        depth(),
        FeatureValue::Boolean(true),
        FeatureValue::Boolean(true),
    );
    assert!(matches!(
        boolean_inventory.observation("inventory", ObservationMintKind::DecimalDepth),
        Err(MarketError::Malformed {
            what: "inventory",
            reason: "boolean cannot mint decimal depth",
        })
    ));
    assert!(matches!(
        boolean_inventory.require_observed_book_and_fills(),
        Err(MarketError::Malformed {
            what: "inventory",
            reason: "boolean cannot mint decimal depth",
        })
    ));
}

#[test]
fn missing_inventory_is_missing_input_not_malformed() {
    let missing_inventory = snapshot(
        depth(),
        FeatureValue::Boolean(true),
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    assert_eq!(
        missing_inventory
            .observation("inventory", ObservationMintKind::DecimalDepth)
            .unwrap(),
        ObservationStatus::Missing(MissingReason::NotObserved)
    );
    assert!(matches!(
        missing_inventory.require_observed_book_and_fills(),
        Err(MarketError::MissingInput { name: "inventory" })
    ));
}

#[test]
fn decimal_fills_cannot_mint_boolean_presence_proof() {
    let snap = snapshot(depth(), depth(), depth());
    assert!(matches!(
        snap.require_observed_book_and_fills(),
        Err(MarketError::Malformed {
            what: "fills",
            reason: "decimal depth cannot mint boolean presence",
        })
    ));
}

#[test]
fn boolean_false_fills_withhold_instead_of_admitting_absent_depth() {
    let snap = snapshot(depth(), FeatureValue::Boolean(false), depth());
    assert!(matches!(
        snap.require_observed_book_and_fills(),
        Err(MarketError::MissingInput { name: "fills" })
    ));
    assert_eq!(
        snap.observation("fills", ObservationMintKind::BooleanPresence)
            .unwrap(),
        ObservationStatus::Missing(MissingReason::NotObserved)
    );
}
