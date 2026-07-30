use domain_types::{Address, BlockHeight, ChainId, LiquidationId, TwapId, ValueError};

#[test]
fn ids_reject_empty_and_whitespace_padded_values() {
    for value in ["", " chain", "chain ", "\tchain", "chain\n"] {
        assert_eq!(ChainId::new(value), Err(ValueError::Invalid), "{value:?}");
    }
}

#[test]
fn block_height_preserves_its_unsigned_value() {
    assert_eq!(BlockHeight::new(42).get(), 42);
}

#[test]
fn address_round_trips_exactly_at_the_twenty_byte_boundary() {
    let bytes = [0xabu8; 20];
    let address = Address::from_bytes(bytes);
    assert_eq!(address.as_bytes(), &bytes);
    assert_eq!(
        Address::parse_api(&address.to_api_string()).unwrap(),
        address
    );
}

#[test]
fn api_addresses_format_lowercase_hex() {
    let address = Address::from_bytes([0xafu8; 20]);
    assert_eq!(address.to_api_string(), "0x".to_owned() + &"af".repeat(20));
}

#[test]
fn addresses_reject_noncanonical_and_malformed_api_values() {
    for value in [
        "",
        "0x",
        "0x12",
        "12aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "0xgggggggggggggggggggggggggggggggggggggggg",
        "0Xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        assert_eq!(
            Address::parse_api(value),
            Err(ValueError::Invalid),
            "{value}"
        );
    }
}

#[test]
fn id_and_address_serde_revalidate_external_values() {
    assert!(serde_json::from_str::<ChainId>("\"chain-a\"").is_ok());
    assert!(serde_json::from_str::<ChainId>("\" chain-a\"").is_err());
    assert!(
        serde_json::from_str::<Address>("\"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"").is_ok()
    );
    assert!(
        serde_json::from_str::<Address>("\"0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"").is_err()
    );
}

#[test]
fn liquidation_ids_are_strict_canonical_domain_ids() {
    let liquidation_id = LiquidationId::new("liquidation-42").unwrap();
    assert_eq!(liquidation_id.as_str(), "liquidation-42");
    assert_eq!(liquidation_id.to_string(), "liquidation-42");
    assert_eq!(
        serde_json::from_str::<LiquidationId>(&serde_json::to_string(&liquidation_id).unwrap())
            .unwrap(),
        liquidation_id
    );

    for invalid in ["", " liquidation-42", "liquidation-42 ", "\tliquidation-42"] {
        assert_eq!(
            LiquidationId::new(invalid),
            Err(ValueError::Invalid),
            "{invalid:?}"
        );
    }
    assert!(serde_json::from_str::<LiquidationId>("\" liquidation-42\"").is_err());
}

#[test]
fn twap_ids_preserve_the_documented_unsigned_numeric_identity() {
    let id = TwapId::new(12_212_201_265);
    assert_eq!(id.get(), 12_212_201_265);
    assert_eq!(id.to_string(), "12212201265");
    assert_eq!(
        serde_json::from_str::<TwapId>(&serde_json::to_string(&id).unwrap()).unwrap(),
        id
    );
    assert!(serde_json::from_str::<TwapId>("\"12212201265\"").is_err());
    assert!(serde_json::from_str::<TwapId>("-1").is_err());
}
