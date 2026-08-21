use canonical_events::EventKind;
use canonical_ledger::{
    AccountFactRecordV1, AccountModeCurrentRecordV1, AccountQuantityFlowCurrentRecordV1,
    AccountQuantityFlowScopeV1, AccountQuoteFlowCurrentRecordV1, AccountQuoteFlowScopeV1,
    AccountStateError, AccountVaultRelationCurrentRecordV1, CanonicalAccountReducerV1,
    LeverageCurrentRecordV1, MarginModeCurrentRecordV1, SubaccountMasterCurrentRecordV1,
    VaultPrincipalFlowCurrentRecordV1, VaultShareFlowCurrentRecordV1,
};
use domain_types::{
    AccountAbstractionModeV1, Address, AssetId, BlockHeight, EventId, Leverage, MarginModeV1,
    MarketId, Quantity, QuoteAmount, VaultId,
};

const ACCOUNT_A: &str = "0x1111111111111111111111111111111111111111";
const ACCOUNT_B: &str = "0x2222222222222222222222222222222222222222";
const ACCOUNT_C: &str = "0x3333333333333333333333333333333333333333";
const HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const VERSION: &str = "hyperliquid-alpha-desk-canonical-account@1.0.0";

#[test]
fn account_record_api_freezes_the_reducer_version() {
    assert_eq!(CanonicalAccountReducerV1::VERSION, VERSION);
    assert_eq!(std::mem::size_of::<CanonicalAccountReducerV1>(), 0);

    let _public_types = (
        std::mem::size_of::<AccountFactRecordV1>(),
        std::mem::size_of::<AccountQuantityFlowScopeV1>(),
        std::mem::size_of::<AccountQuantityFlowCurrentRecordV1>(),
        std::mem::size_of::<AccountQuoteFlowScopeV1>(),
        std::mem::size_of::<AccountQuoteFlowCurrentRecordV1>(),
        std::mem::size_of::<VaultPrincipalFlowCurrentRecordV1>(),
        std::mem::size_of::<VaultShareFlowCurrentRecordV1>(),
        std::mem::size_of::<SubaccountMasterCurrentRecordV1>(),
        std::mem::size_of::<AccountVaultRelationCurrentRecordV1>(),
        std::mem::size_of::<AccountModeCurrentRecordV1>(),
        std::mem::size_of::<MarginModeCurrentRecordV1>(),
        std::mem::size_of::<LeverageCurrentRecordV1>(),
        std::mem::size_of::<AccountStateError>(),
    );
}

#[test]
fn all_fifteen_fact_shapes_decode_without_reordering_or_deduplicating_identities() {
    let cases = [
        (
            "DepositCredited",
            format!(r#"["{ACCOUNT_A}"]"#),
            "[]",
            Some("USDC"),
            None,
            EventKind::DepositCredited,
        ),
        (
            "WithdrawalDebited",
            format!(r#"["{ACCOUNT_A}"]"#),
            "[]",
            Some("USDC"),
            None,
            EventKind::WithdrawalDebited,
        ),
        (
            "SpotTransfer",
            format!(r#"["{ACCOUNT_A}","{ACCOUNT_B}"]"#),
            "[]",
            Some("USDC"),
            None,
            EventKind::SpotTransfer,
        ),
        (
            "PerpTransfer",
            format!(r#"["{ACCOUNT_A}","{ACCOUNT_B}"]"#),
            "[]",
            None,
            None,
            EventKind::PerpTransfer,
        ),
        (
            "SubaccountTransfer",
            format!(r#"["{ACCOUNT_A}","{ACCOUNT_A}","{ACCOUNT_B}"]"#),
            "[]",
            Some("USDC"),
            None,
            EventKind::SubaccountTransfer,
        ),
        (
            "VaultDeposit",
            format!(r#"["{ACCOUNT_A}"]"#),
            "[]",
            None,
            Some("vault-a"),
            EventKind::VaultDeposit,
        ),
        (
            "VaultWithdrawal",
            format!(r#"["{ACCOUNT_A}"]"#),
            "[]",
            None,
            Some("vault-a"),
            EventKind::VaultWithdrawal,
        ),
        (
            "FeeCharged",
            format!(r#"["{ACCOUNT_A}"]"#),
            "[]",
            Some("USDC"),
            None,
            EventKind::FeeCharged,
        ),
        (
            "BuilderFeeCharged",
            format!(r#"["{ACCOUNT_A}","{ACCOUNT_B}"]"#),
            "[]",
            Some("USDC"),
            None,
            EventKind::BuilderFeeCharged,
        ),
        (
            "FundingPaid",
            format!(r#"["{ACCOUNT_A}"]"#),
            r#"["perp:BTC"]"#,
            None,
            None,
            EventKind::FundingPaid,
        ),
        (
            "FundingReceived",
            format!(r#"["{ACCOUNT_A}"]"#),
            r#"["perp:BTC"]"#,
            None,
            None,
            EventKind::FundingReceived,
        ),
        (
            "ReferralReward",
            format!(r#"["{ACCOUNT_A}","{ACCOUNT_B}"]"#),
            "[]",
            Some("USDC"),
            None,
            EventKind::ReferralReward,
        ),
        (
            "AccountModeChanged",
            format!(r#"["{ACCOUNT_A}"]"#),
            "[]",
            None,
            None,
            EventKind::AccountModeChanged,
        ),
        (
            "MarginModeChanged",
            format!(r#"["{ACCOUNT_A}"]"#),
            r#"["perp:BTC"]"#,
            None,
            None,
            EventKind::MarginModeChanged,
        ),
        (
            "LeverageChanged",
            format!(r#"["{ACCOUNT_A}"]"#),
            r#"["perp:BTC"]"#,
            None,
            None,
            EventKind::LeverageChanged,
        ),
    ];

    for (index, (kind, accounts, markets, asset, vault, expected_kind)) in
        cases.into_iter().enumerate()
    {
        let event_id = format!("event-{index}");
        let bytes = fact_bytes(&event_id, kind, &accounts, markets, asset, vault, VERSION);
        let record = AccountFactRecordV1::decode(&bytes).unwrap();
        let key = AccountFactRecordV1::state_key(&EventId::new(event_id).unwrap()).unwrap();
        assert_eq!(
            AccountFactRecordV1::decode_at(&key, &bytes).unwrap(),
            record
        );
        assert_eq!(record.event_kind(), expected_kind);
        assert_eq!(record.rule_version(), VERSION);
        assert_eq!(record.block_height(), BlockHeight::new(7));
        assert_eq!(record.payload_hash(), [0; 32]);
    }

    let subaccount = AccountFactRecordV1::decode(&fact_bytes(
        "subaccount-event",
        "SubaccountTransfer",
        &format!(r#"["{ACCOUNT_A}","{ACCOUNT_A}","{ACCOUNT_B}"]"#),
        "[]",
        Some("USDC"),
        None,
        VERSION,
    ))
    .unwrap();
    assert_eq!(
        subaccount.account_ids(),
        &[
            Address::parse_api(ACCOUNT_A).unwrap(),
            Address::parse_api(ACCOUNT_A).unwrap(),
            Address::parse_api(ACCOUNT_B).unwrap(),
        ]
    );
    assert!(subaccount.market_ids().is_empty());
    assert_eq!(subaccount.asset_id(), Some(&AssetId::new("USDC").unwrap()));
    assert_eq!(subaccount.vault_id(), None);
}

#[test]
fn quantity_and_quote_flow_records_are_unit_separated_key_bound_and_scale_strict() {
    let quantity_bytes = quantity_flow_bytes("external_asset", Some("USDC"), None, "12.50", "2.00");
    let quantity = AccountQuantityFlowCurrentRecordV1::decode(&quantity_bytes).unwrap();
    let quantity_scope = AccountQuantityFlowScopeV1::ExternalAsset {
        asset_id: AssetId::new("USDC").unwrap(),
    };
    let quantity_key = AccountQuantityFlowCurrentRecordV1::state_key(
        &Address::parse_api(ACCOUNT_A).unwrap(),
        &quantity_scope,
    )
    .unwrap();
    assert_eq!(
        AccountQuantityFlowCurrentRecordV1::decode_at(&quantity_key, &quantity_bytes).unwrap(),
        quantity
    );
    assert_eq!(
        quantity.account_id(),
        Address::parse_api(ACCOUNT_A).unwrap()
    );
    assert_eq!(quantity.scope(), &quantity_scope);
    assert_eq!(quantity.credits(), quantity_value("12.50"));
    assert_eq!(quantity.debits(), quantity_value("2.00"));
    assert_eq!(
        quantity.last_event_id(),
        &EventId::new("event-last").unwrap()
    );
    assert_eq!(quantity.last_block_height(), BlockHeight::new(9));

    let quote_bytes = quote_flow_bytes("market_funding", Some("perp:BTC"), None, "4.000", "1.250");
    let quote = AccountQuoteFlowCurrentRecordV1::decode(&quote_bytes).unwrap();
    let quote_scope = AccountQuoteFlowScopeV1::MarketFunding {
        market_id: MarketId::new("perp:BTC").unwrap(),
    };
    let quote_key = AccountQuoteFlowCurrentRecordV1::state_key(
        &Address::parse_api(ACCOUNT_A).unwrap(),
        &quote_scope,
    )
    .unwrap();
    assert_eq!(
        AccountQuoteFlowCurrentRecordV1::decode_at(&quote_key, &quote_bytes).unwrap(),
        quote
    );
    assert_eq!(quote.scope(), &quote_scope);
    assert_eq!(quote.credits(), quote_value("4.000"));
    assert_eq!(quote.debits(), quote_value("1.250"));

    assert_eq!(
        AccountQuantityFlowCurrentRecordV1::decode(&quote_bytes),
        Err(AccountStateError::Codec)
    );
    assert_eq!(
        AccountQuoteFlowCurrentRecordV1::decode(&quantity_bytes),
        Err(AccountStateError::Codec)
    );
    assert_eq!(
        AccountQuantityFlowCurrentRecordV1::decode(&quantity_flow_bytes(
            "external_asset",
            Some("USDC"),
            None,
            "-1.0",
            "0.0",
        )),
        Err(AccountStateError::InvalidRecord)
    );
    assert_eq!(
        AccountQuoteFlowCurrentRecordV1::decode(&quote_flow_bytes(
            "default_perp_quote",
            None,
            None,
            "1.0",
            "0.00",
        )),
        Err(AccountStateError::InvalidRecord)
    );
}

#[test]
fn every_quantity_and_quote_scope_has_an_unambiguous_key_identity() {
    let account = Address::parse_api(ACCOUNT_A).unwrap();
    let asset = AssetId::new("USDC").unwrap();
    let vault = VaultId::new("vault-a").unwrap();
    let quantity_scopes = [
        AccountQuantityFlowScopeV1::ExternalAsset {
            asset_id: asset.clone(),
        },
        AccountQuantityFlowScopeV1::SpotTransferAsset {
            asset_id: asset.clone(),
        },
        AccountQuantityFlowScopeV1::SubaccountTransferAsset {
            asset_id: asset.clone(),
        },
        AccountQuantityFlowScopeV1::FeeAsset {
            asset_id: asset.clone(),
        },
        AccountQuantityFlowScopeV1::BuilderFeeAsset {
            asset_id: asset.clone(),
        },
        AccountQuantityFlowScopeV1::ReferralRewardAsset {
            asset_id: asset.clone(),
        },
        AccountQuantityFlowScopeV1::VaultShares {
            vault_id: vault.clone(),
        },
        AccountQuantityFlowScopeV1::SpotGenesisAsset { asset_id: asset },
    ];
    let quantity_keys = quantity_scopes
        .iter()
        .map(|scope| AccountQuantityFlowCurrentRecordV1::state_key(&account, scope).unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(quantity_keys.len(), quantity_scopes.len());
    assert!(
        quantity_keys
            .iter()
            .all(|key| key.namespace() == "account-quantity-flow-current.v1")
    );

    let quote_scopes = [
        AccountQuoteFlowScopeV1::DefaultPerpQuote,
        AccountQuoteFlowScopeV1::MarketFunding {
            market_id: MarketId::new("perp:BTC").unwrap(),
        },
        AccountQuoteFlowScopeV1::VaultPrincipal { vault_id: vault },
        AccountQuoteFlowScopeV1::SpotClassQuote,
        AccountQuoteFlowScopeV1::RewardClaimedQuote,
    ];
    let quote_keys = quote_scopes
        .iter()
        .map(|scope| AccountQuoteFlowCurrentRecordV1::state_key(&account, scope).unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(quote_keys.len(), quote_scopes.len());
    assert!(
        quote_keys
            .iter()
            .all(|key| key.namespace() == "account-quote-flow-current.v1")
    );
}

#[test]
fn vault_records_expose_observed_flows_without_collapsing_principal_and_shares() {
    let principal_bytes = vault_principal_bytes("10.00", "3.00");
    let principal = VaultPrincipalFlowCurrentRecordV1::decode(&principal_bytes).unwrap();
    let principal_key =
        VaultPrincipalFlowCurrentRecordV1::state_key(&VaultId::new("vault-a").unwrap()).unwrap();
    assert_eq!(
        VaultPrincipalFlowCurrentRecordV1::decode_at(&principal_key, &principal_bytes).unwrap(),
        principal
    );
    assert_eq!(principal.vault_id(), &VaultId::new("vault-a").unwrap());
    assert_eq!(principal.deposits(), quote_value("10.00"));
    assert_eq!(principal.withdrawals(), quote_value("3.00"));
    assert_eq!(
        principal.last_event_id(),
        &EventId::new("event-last").unwrap()
    );
    assert_eq!(principal.last_block_height(), BlockHeight::new(9));

    let shares_bytes = vault_share_bytes("7.000", "2.000");
    let shares = VaultShareFlowCurrentRecordV1::decode(&shares_bytes).unwrap();
    let shares_key =
        VaultShareFlowCurrentRecordV1::state_key(&VaultId::new("vault-a").unwrap()).unwrap();
    assert_eq!(
        VaultShareFlowCurrentRecordV1::decode_at(&shares_key, &shares_bytes).unwrap(),
        shares
    );
    assert_eq!(shares.shares_issued(), quantity_value("7.000"));
    assert_eq!(shares.shares_redeemed(), quantity_value("2.000"));
    assert_eq!(
        VaultPrincipalFlowCurrentRecordV1::decode(&vault_principal_bytes("-1.0", "0.0")),
        Err(AccountStateError::InvalidRecord)
    );
    assert_eq!(
        VaultShareFlowCurrentRecordV1::decode(&vault_share_bytes("1.0", "0.00")),
        Err(AccountStateError::InvalidRecord)
    );
}

#[test]
fn relation_records_are_direct_observations_with_distinct_endpoints_and_monotonic_heights() {
    let subaccount_bytes = subaccount_relation_bytes(5, 8);
    let subaccount = SubaccountMasterCurrentRecordV1::decode(&subaccount_bytes).unwrap();
    let subaccount_key =
        SubaccountMasterCurrentRecordV1::state_key(&Address::parse_api(ACCOUNT_B).unwrap())
            .unwrap();
    assert_eq!(
        SubaccountMasterCurrentRecordV1::decode_at(&subaccount_key, &subaccount_bytes).unwrap(),
        subaccount
    );
    assert_eq!(
        subaccount.subaccount_id(),
        Address::parse_api(ACCOUNT_B).unwrap()
    );
    assert_eq!(
        subaccount.master_account_id(),
        Address::parse_api(ACCOUNT_A).unwrap()
    );
    assert_eq!(
        subaccount.first_event_id(),
        &EventId::new("event-first").unwrap()
    );
    assert_eq!(
        subaccount.last_event_id(),
        &EventId::new("event-last").unwrap()
    );
    assert_eq!(subaccount.first_block_height(), BlockHeight::new(5));
    assert_eq!(subaccount.last_block_height(), BlockHeight::new(8));

    let vault_bytes = account_vault_relation_bytes(5, 8);
    let vault_relation = AccountVaultRelationCurrentRecordV1::decode(&vault_bytes).unwrap();
    let vault_key = AccountVaultRelationCurrentRecordV1::state_key(
        &Address::parse_api(ACCOUNT_A).unwrap(),
        &VaultId::new("vault-a").unwrap(),
    )
    .unwrap();
    assert_eq!(
        AccountVaultRelationCurrentRecordV1::decode_at(&vault_key, &vault_bytes).unwrap(),
        vault_relation
    );
    assert_eq!(
        vault_relation.account_id(),
        Address::parse_api(ACCOUNT_A).unwrap()
    );
    assert_eq!(vault_relation.vault_id(), &VaultId::new("vault-a").unwrap());
    assert_eq!(vault_relation.first_block_height(), BlockHeight::new(5));
    assert_eq!(vault_relation.last_block_height(), BlockHeight::new(8));

    let self_edge = subaccount_relation_bytes_for(ACCOUNT_A, ACCOUNT_A, 5, 8);
    assert_eq!(
        SubaccountMasterCurrentRecordV1::decode(&self_edge),
        Err(AccountStateError::InvalidRecord)
    );
    assert_eq!(
        SubaccountMasterCurrentRecordV1::decode(&subaccount_relation_bytes(9, 8)),
        Err(AccountStateError::InvalidRecord)
    );
    assert_eq!(
        AccountVaultRelationCurrentRecordV1::decode(&account_vault_relation_bytes(9, 8)),
        Err(AccountStateError::InvalidRecord)
    );
    assert_eq!(
        AccountVaultRelationCurrentRecordV1::decode(&account_vault_relation_bytes_for(
            ACCOUNT_A, 5, 8,
        )),
        Err(AccountStateError::InvalidRecord)
    );
}

#[test]
fn mode_and_leverage_records_bind_predecessors_without_forbidding_later_cycles() {
    let account_bytes = account_mode_bytes("standard", "standard", 5, 8);
    let account_mode = AccountModeCurrentRecordV1::decode(&account_bytes).unwrap();
    let account_key =
        AccountModeCurrentRecordV1::state_key(&Address::parse_api(ACCOUNT_A).unwrap()).unwrap();
    assert_eq!(
        AccountModeCurrentRecordV1::decode_at(&account_key, &account_bytes).unwrap(),
        account_mode
    );
    assert_eq!(
        account_mode.initial_previous(),
        AccountAbstractionModeV1::Standard
    );
    assert_eq!(account_mode.current(), AccountAbstractionModeV1::Standard);

    let margin_bytes = margin_mode_bytes("cross", "cross", 5, 8);
    let margin = MarginModeCurrentRecordV1::decode(&margin_bytes).unwrap();
    let margin_key = MarginModeCurrentRecordV1::state_key(
        &Address::parse_api(ACCOUNT_A).unwrap(),
        &MarketId::new("perp:BTC").unwrap(),
    )
    .unwrap();
    assert_eq!(
        MarginModeCurrentRecordV1::decode_at(&margin_key, &margin_bytes).unwrap(),
        margin
    );
    assert_eq!(margin.initial_previous(), MarginModeV1::Cross);
    assert_eq!(margin.current(), MarginModeV1::Cross);

    let valid_leverage_bytes = leverage_bytes("3.00", "3.00", 5, 8);
    let leverage = LeverageCurrentRecordV1::decode(&valid_leverage_bytes).unwrap();
    let leverage_key = LeverageCurrentRecordV1::state_key(
        &Address::parse_api(ACCOUNT_A).unwrap(),
        &MarketId::new("perp:BTC").unwrap(),
    )
    .unwrap();
    assert_eq!(
        LeverageCurrentRecordV1::decode_at(&leverage_key, &valid_leverage_bytes).unwrap(),
        leverage
    );
    assert_eq!(leverage.initial_previous(), leverage_value("3.00"));
    assert_eq!(leverage.current(), leverage_value("3.00"));
    assert_eq!(
        leverage.first_event_id(),
        &EventId::new("event-first").unwrap()
    );
    assert_eq!(
        leverage.last_event_id(),
        &EventId::new("event-last").unwrap()
    );
    assert_eq!(leverage.first_block_height(), BlockHeight::new(5));
    assert_eq!(leverage.last_block_height(), BlockHeight::new(8));

    assert_eq!(
        AccountModeCurrentRecordV1::decode(&account_mode_bytes("standard", "unified", 9, 8)),
        Err(AccountStateError::InvalidRecord)
    );
    assert_eq!(
        MarginModeCurrentRecordV1::decode(&margin_mode_bytes("cross", "isolated", 9, 8)),
        Err(AccountStateError::InvalidRecord)
    );
    assert_eq!(
        LeverageCurrentRecordV1::decode(&leverage_bytes("0", "3", 5, 8)),
        Err(AccountStateError::InvalidRecord)
    );
    assert_eq!(
        LeverageCurrentRecordV1::decode(&leverage_bytes("3", "-1", 5, 8)),
        Err(AccountStateError::InvalidRecord)
    );
    assert_eq!(
        LeverageCurrentRecordV1::decode(&leverage_bytes("3", "4", 9, 8)),
        Err(AccountStateError::InvalidRecord)
    );
}

#[test]
fn codecs_reject_noncanonical_unknown_duplicate_trailing_and_oversized_bytes() {
    let canonical = fact_bytes(
        "event-canonical",
        "DepositCredited",
        &format!(r#"["{ACCOUNT_A}"]"#),
        "[]",
        Some("USDC"),
        None,
        VERSION,
    );
    assert!(AccountFactRecordV1::decode(&canonical).is_ok());

    let reordered = format!(
        r#"{{"event_id":"event-canonical","schema":"hyperliquid-alpha-desk/account-fact/v1","event_kind":"DepositCredited","account_ids":["{ACCOUNT_A}"],"market_ids":[],"asset_id":"USDC","vault_id":null,"block_height":7,"payload_blake3":"{HASH}","rule_version":"{VERSION}"}}"#
    )
    .into_bytes();
    assert_eq!(
        AccountFactRecordV1::decode(&reordered),
        Err(AccountStateError::NonCanonical)
    );
    assert_eq!(
        AccountFactRecordV1::decode(&insert_before_close(&canonical, r#","unknown_field":true"#,)),
        Err(AccountStateError::Codec)
    );
    assert_eq!(
        AccountFactRecordV1::decode(&insert_before_close(
            &canonical,
            r#","schema":"hyperliquid-alpha-desk/account-fact/v1""#,
        )),
        Err(AccountStateError::Codec)
    );
    let mut trailing = canonical.clone();
    trailing.extend_from_slice(b"{}");
    assert_eq!(
        AccountFactRecordV1::decode(&trailing),
        Err(AccountStateError::Codec)
    );
    let mut whitespace = vec![b' '];
    whitespace.extend_from_slice(&canonical);
    assert_eq!(
        AccountFactRecordV1::decode(&whitespace),
        Err(AccountStateError::NonCanonical)
    );
    assert_eq!(
        AccountFactRecordV1::decode(&vec![b'x'; 16 * 1024 + 1]),
        Err(AccountStateError::LimitExceeded)
    );
    assert_eq!(
        AccountFactRecordV1::decode(&vec![b'x'; 70_000]),
        Err(AccountStateError::LimitExceeded)
    );

    let one_byte_id = fact_bytes(
        "e",
        "DepositCredited",
        &format!(r#"["{ACCOUNT_A}"]"#),
        "[]",
        Some("USDC"),
        None,
        VERSION,
    );
    let padding = (16 * 1024) - one_byte_id.len() + 1;
    let exact = fact_bytes(
        &"e".repeat(padding),
        "DepositCredited",
        &format!(r#"["{ACCOUNT_A}"]"#),
        "[]",
        Some("USDC"),
        None,
        VERSION,
    );
    assert_eq!(exact.len(), 16 * 1024);
    assert!(AccountFactRecordV1::decode(&exact).is_ok());
}

#[test]
fn every_record_rejects_a_valid_value_at_the_wrong_typed_key() {
    let account = Address::parse_api(ACCOUNT_A).unwrap();
    let other_account = Address::parse_api(ACCOUNT_B).unwrap();
    let vault = VaultId::new("vault-a").unwrap();
    let other_vault = VaultId::new("vault-b").unwrap();
    let market = MarketId::new("perp:BTC").unwrap();
    let other_market = MarketId::new("perp:ETH").unwrap();
    let asset = AssetId::new("USDC").unwrap();
    let fact = fact_bytes(
        "event-fact",
        "DepositCredited",
        &format!(r#"["{ACCOUNT_A}"]"#),
        "[]",
        Some("USDC"),
        None,
        VERSION,
    );
    assert_eq!(
        AccountFactRecordV1::decode_at(
            &AccountFactRecordV1::state_key(&EventId::new("wrong").unwrap()).unwrap(),
            &fact,
        ),
        Err(AccountStateError::KeyMismatch)
    );

    let quantity = quantity_flow_bytes("external_asset", Some("USDC"), None, "1.0", "0.0");
    assert_eq!(
        AccountQuantityFlowCurrentRecordV1::decode_at(
            &AccountQuantityFlowCurrentRecordV1::state_key(
                &other_account,
                &AccountQuantityFlowScopeV1::ExternalAsset {
                    asset_id: asset.clone(),
                },
            )
            .unwrap(),
            &quantity,
        ),
        Err(AccountStateError::KeyMismatch)
    );

    let quote = quote_flow_bytes("default_perp_quote", None, None, "1.0", "0.0");
    assert_eq!(
        AccountQuoteFlowCurrentRecordV1::decode_at(
            &AccountQuoteFlowCurrentRecordV1::state_key(
                &other_account,
                &AccountQuoteFlowScopeV1::DefaultPerpQuote,
            )
            .unwrap(),
            &quote,
        ),
        Err(AccountStateError::KeyMismatch)
    );
    assert_eq!(
        VaultPrincipalFlowCurrentRecordV1::decode_at(
            &VaultPrincipalFlowCurrentRecordV1::state_key(&other_vault).unwrap(),
            &vault_principal_bytes("1.0", "0.0"),
        ),
        Err(AccountStateError::KeyMismatch)
    );
    assert_eq!(
        VaultShareFlowCurrentRecordV1::decode_at(
            &VaultShareFlowCurrentRecordV1::state_key(&other_vault).unwrap(),
            &vault_share_bytes("1.0", "0.0"),
        ),
        Err(AccountStateError::KeyMismatch)
    );
    assert_eq!(
        SubaccountMasterCurrentRecordV1::decode_at(
            &SubaccountMasterCurrentRecordV1::state_key(&Address::parse_api(ACCOUNT_C).unwrap())
                .unwrap(),
            &subaccount_relation_bytes(5, 8),
        ),
        Err(AccountStateError::KeyMismatch)
    );
    assert_eq!(
        AccountVaultRelationCurrentRecordV1::decode_at(
            &AccountVaultRelationCurrentRecordV1::state_key(&account, &other_vault).unwrap(),
            &account_vault_relation_bytes(5, 8),
        ),
        Err(AccountStateError::KeyMismatch)
    );
    assert_eq!(
        AccountModeCurrentRecordV1::decode_at(
            &AccountModeCurrentRecordV1::state_key(&other_account).unwrap(),
            &account_mode_bytes("standard", "unified", 5, 8),
        ),
        Err(AccountStateError::KeyMismatch)
    );
    assert_eq!(
        MarginModeCurrentRecordV1::decode_at(
            &MarginModeCurrentRecordV1::state_key(&account, &other_market).unwrap(),
            &margin_mode_bytes("cross", "isolated", 5, 8),
        ),
        Err(AccountStateError::KeyMismatch)
    );
    assert_eq!(
        LeverageCurrentRecordV1::decode_at(
            &LeverageCurrentRecordV1::state_key(&account, &other_market).unwrap(),
            &leverage_bytes("3", "4", 5, 8),
        ),
        Err(AccountStateError::KeyMismatch)
    );

    let _all_valid_identities = (vault, market, asset);
}

#[test]
fn key_builders_accept_the_exact_64_kib_frame_and_fail_closed_above_it_without_panicking() {
    const MAX_KEY_BYTES: usize = 64 * 1024;
    const FRAME_BYTES: usize = 8;

    let exact_event = EventId::new("e".repeat(MAX_KEY_BYTES - FRAME_BYTES)).unwrap();
    let exact_key = AccountFactRecordV1::state_key(&exact_event).unwrap();
    assert_eq!(exact_key.key().len(), MAX_KEY_BYTES);

    let one_over = EventId::new("e".repeat(MAX_KEY_BYTES - FRAME_BYTES + 1)).unwrap();
    assert_eq!(
        AccountFactRecordV1::state_key(&one_over),
        Err(AccountStateError::InvalidKey)
    );
    let seventy_k = EventId::new("e".repeat(70_000)).unwrap();
    assert_eq!(
        AccountFactRecordV1::state_key(&seventy_k),
        Err(AccountStateError::InvalidKey)
    );
}

#[test]
fn invalid_schemas_fact_shapes_and_scope_identity_shapes_fail_closed() {
    assert_eq!(
        AccountFactRecordV1::decode(&fact_bytes(
            "event",
            "OrderFilled",
            &format!(r#"["{ACCOUNT_A}"]"#),
            "[]",
            None,
            None,
            VERSION,
        )),
        Err(AccountStateError::InvalidRecord)
    );
    assert_eq!(
        AccountFactRecordV1::decode(&fact_bytes(
            "event",
            "FundingPaid",
            &format!(r#"["{ACCOUNT_A}"]"#),
            "[]",
            None,
            None,
            VERSION,
        )),
        Err(AccountStateError::InvalidRecord)
    );
    assert_eq!(
        AccountFactRecordV1::decode(&fact_bytes(
            "event",
            "DepositCredited",
            &format!(r#"["{ACCOUNT_A}"]"#),
            "[]",
            None,
            None,
            VERSION,
        )),
        Err(AccountStateError::InvalidRecord)
    );
    assert_eq!(
        AccountFactRecordV1::decode(&fact_bytes(
            "event",
            "DepositCredited",
            &format!(r#"["{ACCOUNT_A}"]"#),
            "[]",
            Some("USDC"),
            None,
            "wrong-version",
        )),
        Err(AccountStateError::InvalidRecord)
    );

    let wrong_schema = replace_once(
        &fact_bytes(
            "event",
            "DepositCredited",
            &format!(r#"["{ACCOUNT_A}"]"#),
            "[]",
            Some("USDC"),
            None,
            VERSION,
        ),
        "hyperliquid-alpha-desk/account-fact/v1",
        "hyperliquid-alpha-desk/account-fact/v2",
    );
    assert_eq!(
        AccountFactRecordV1::decode(&wrong_schema),
        Err(AccountStateError::InvalidRecord)
    );
    assert_eq!(
        AccountQuantityFlowCurrentRecordV1::decode(&quantity_flow_bytes(
            "vault_shares",
            Some("USDC"),
            None,
            "1",
            "0",
        )),
        Err(AccountStateError::InvalidRecord)
    );
    assert_eq!(
        AccountQuoteFlowCurrentRecordV1::decode(&quote_flow_bytes(
            "market_funding",
            None,
            None,
            "1",
            "0",
        )),
        Err(AccountStateError::InvalidRecord)
    );
}

#[test]
fn account_error_reason_codes_are_stable_and_distinct() {
    let cases = [
        (
            AccountStateError::InvalidKey,
            "account_state.codec.invalid_key",
        ),
        (AccountStateError::Codec, "account_state.codec.decode"),
        (
            AccountStateError::NonCanonical,
            "account_state.codec.noncanonical",
        ),
        (
            AccountStateError::InvalidRecord,
            "account_state.codec.invalid_record",
        ),
        (
            AccountStateError::KeyMismatch,
            "account_state.codec.key_mismatch",
        ),
        (
            AccountStateError::LimitExceeded,
            "account_state.codec.limit_exceeded",
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(error.reason_code(), expected);
    }
}

fn fact_bytes(
    event_id: &str,
    event_kind: &str,
    account_ids: &str,
    market_ids: &str,
    asset_id: Option<&str>,
    vault_id: Option<&str>,
    rule_version: &str,
) -> Vec<u8> {
    format!(
        r#"{{"schema":"hyperliquid-alpha-desk/account-fact/v1","event_id":"{event_id}","event_kind":"{event_kind}","account_ids":{account_ids},"market_ids":{market_ids},"asset_id":{},"vault_id":{},"block_height":7,"payload_blake3":"{HASH}","rule_version":"{rule_version}"}}"#,
        json_optional(asset_id),
        json_optional(vault_id),
    )
    .into_bytes()
}

fn quantity_flow_bytes(
    scope: &str,
    asset_id: Option<&str>,
    vault_id: Option<&str>,
    credits: &str,
    debits: &str,
) -> Vec<u8> {
    format!(
        r#"{{"schema":"hyperliquid-alpha-desk/account-quantity-flow-current/v1","account_id":"{ACCOUNT_A}","scope":"{scope}","asset_id":{},"vault_id":{},"credits":"{credits}","debits":"{debits}","last_event_id":"event-last","last_block_height":9}}"#,
        json_optional(asset_id),
        json_optional(vault_id),
    )
    .into_bytes()
}

fn quote_flow_bytes(
    scope: &str,
    market_id: Option<&str>,
    vault_id: Option<&str>,
    credits: &str,
    debits: &str,
) -> Vec<u8> {
    format!(
        r#"{{"schema":"hyperliquid-alpha-desk/account-quote-flow-current/v1","account_id":"{ACCOUNT_A}","scope":"{scope}","market_id":{},"vault_id":{},"credits":"{credits}","debits":"{debits}","last_event_id":"event-last","last_block_height":9}}"#,
        json_optional(market_id),
        json_optional(vault_id),
    )
    .into_bytes()
}

fn vault_principal_bytes(deposits: &str, withdrawals: &str) -> Vec<u8> {
    format!(
        r#"{{"schema":"hyperliquid-alpha-desk/vault-principal-flow-current/v1","vault_id":"vault-a","deposits":"{deposits}","withdrawals":"{withdrawals}","last_event_id":"event-last","last_block_height":9}}"#
    )
    .into_bytes()
}

fn vault_share_bytes(issued: &str, redeemed: &str) -> Vec<u8> {
    format!(
        r#"{{"schema":"hyperliquid-alpha-desk/vault-share-flow-current/v1","vault_id":"vault-a","shares_issued":"{issued}","shares_redeemed":"{redeemed}","last_event_id":"event-last","last_block_height":9}}"#
    )
    .into_bytes()
}

fn subaccount_relation_bytes(first_height: u64, last_height: u64) -> Vec<u8> {
    subaccount_relation_bytes_for(ACCOUNT_B, ACCOUNT_A, first_height, last_height)
}

fn subaccount_relation_bytes_for(
    subaccount: &str,
    master: &str,
    first_height: u64,
    last_height: u64,
) -> Vec<u8> {
    format!(
        r#"{{"schema":"hyperliquid-alpha-desk/account-subaccount-master/v1","subaccount_id":"{subaccount}","master_account_id":"{master}","first_event_id":"event-first","last_event_id":"event-last","first_block_height":{first_height},"last_block_height":{last_height}}}"#
    )
    .into_bytes()
}

fn account_vault_relation_bytes(first_height: u64, last_height: u64) -> Vec<u8> {
    account_vault_relation_bytes_for("vault-a", first_height, last_height)
}

fn account_vault_relation_bytes_for(
    vault_id: &str,
    first_height: u64,
    last_height: u64,
) -> Vec<u8> {
    format!(
        r#"{{"schema":"hyperliquid-alpha-desk/account-vault-relation/v1","account_id":"{ACCOUNT_A}","vault_id":"{vault_id}","first_event_id":"event-first","last_event_id":"event-last","first_block_height":{first_height},"last_block_height":{last_height}}}"#
    )
    .into_bytes()
}

fn account_mode_bytes(
    initial_previous: &str,
    current: &str,
    first_height: u64,
    last_height: u64,
) -> Vec<u8> {
    format!(
        r#"{{"schema":"hyperliquid-alpha-desk/account-mode-current/v1","account_id":"{ACCOUNT_A}","initial_previous":"{initial_previous}","current":"{current}","first_event_id":"event-first","last_event_id":"event-last","first_block_height":{first_height},"last_block_height":{last_height}}}"#
    )
    .into_bytes()
}

fn margin_mode_bytes(
    initial_previous: &str,
    current: &str,
    first_height: u64,
    last_height: u64,
) -> Vec<u8> {
    format!(
        r#"{{"schema":"hyperliquid-alpha-desk/account-margin-mode-current/v1","account_id":"{ACCOUNT_A}","market_id":"perp:BTC","initial_previous":"{initial_previous}","current":"{current}","first_event_id":"event-first","last_event_id":"event-last","first_block_height":{first_height},"last_block_height":{last_height}}}"#
    )
    .into_bytes()
}

fn leverage_bytes(
    initial_previous: &str,
    current: &str,
    first_height: u64,
    last_height: u64,
) -> Vec<u8> {
    format!(
        r#"{{"schema":"hyperliquid-alpha-desk/account-leverage-current/v1","account_id":"{ACCOUNT_A}","market_id":"perp:BTC","initial_previous":"{initial_previous}","current":"{current}","first_event_id":"event-first","last_event_id":"event-last","first_block_height":{first_height},"last_block_height":{last_height}}}"#
    )
    .into_bytes()
}

fn json_optional(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| format!(r#""{value}""#))
}

fn quantity_value(value: &str) -> Quantity {
    value.parse().unwrap()
}

fn quote_value(value: &str) -> QuoteAmount {
    value.parse().unwrap()
}

fn leverage_value(value: &str) -> Leverage {
    value.parse().unwrap()
}

fn insert_before_close(bytes: &[u8], suffix: &str) -> Vec<u8> {
    let mut value = String::from_utf8(bytes.to_vec()).unwrap();
    value.pop();
    value.push_str(suffix);
    value.push('}');
    value.into_bytes()
}

fn replace_once(bytes: &[u8], from: &str, to: &str) -> Vec<u8> {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .replacen(from, to, 1)
        .into_bytes()
}
