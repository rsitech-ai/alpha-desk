use std::collections::BTreeMap;
use std::str::FromStr;

use api_contracts::{
    WireAccountClassTransfer, WireInternalTransfer, WireRewardClaimed, WireSpotGenesisApplied,
    encode_account_class_transfer, encode_internal_transfer, encode_reward_claimed,
    encode_spot_genesis_applied,
};

use canonical_events::{
    AccountModeChanged, AssetContextUpdated, BlockEnvelope, BuilderFeeCharged,
    CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, DepositCredited, DexCreated,
    EventKind, EventPayload, FeeCharged, FundingPaid, FundingReceived, LeverageChanged,
    LiquidationStarted, MarginModeChanged, MarketCreated, MarketMetadataChanged, PerpTransfer,
    ReferralReward, SourceEvidence, SpotTransfer, SubaccountTransfer, VaultDeposit,
    VaultWithdrawal, WithdrawalDebited,
};
use canonical_ledger::{
    AccountFactRecordV1, AccountModeCurrentRecordV1, AccountQuantityFlowCurrentRecordV1,
    AccountQuantityFlowScopeV1, AccountQuoteFlowCurrentRecordV1, AccountQuoteFlowScopeV1,
    ApplyContext, ApplyOutcome, AssetContextCurrentRecordV1, CanonicalAccountReducerV1,
    CanonicalLedger, CanonicalMarketReducerV1, EventReducer, LedgerLimits, LeverageCurrentRecordV1,
    MarginModeCurrentRecordV1, MarketCurrentRecordV1, ReducerError, StateImage, StateImageLimits,
    StateKey, StateMutation, StateView, SubaccountMasterCurrentRecordV1,
    VaultPrincipalFlowCurrentRecordV1, VaultShareFlowCurrentRecordV1,
};
use domain_types::{
    AccountAbstractionModeV1, Address, AssetId, BlockHeight, ChainId, DexId, FeeRate, FeeTypeV1,
    FundingRate, KnownTime, Leverage, LiquidationId, MarginModeV1, MarketId, Price, ProtocolTime,
    Quantity, QuoteAmount, SourceId, TransactionId, UsdAmount, VaultId,
};

const ACCOUNT_A: Address = Address::from_bytes([0x11; 20]);
const ACCOUNT_B: Address = Address::from_bytes([0x22; 20]);
const ACCOUNT_C: Address = Address::from_bytes([0x33; 20]);
const BUILDER: Address = Address::from_bytes([0x44; 20]);
const REFERRER: Address = Address::from_bytes([0x55; 20]);
const OPERATOR: Address = Address::from_bytes([0x66; 20]);

#[derive(Debug, Clone, Copy, Default)]
struct TestDispatcher {
    market: CanonicalMarketReducerV1,
    account: CanonicalAccountReducerV1,
}

impl EventReducer for TestDispatcher {
    fn reducer_set_version(&self) -> &str {
        "account-test-dispatcher@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        EventReducer::supports(&self.market, event) || EventReducer::supports(&self.account, event)
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        if EventReducer::supports(&self.market, event) {
            EventReducer::reduce(&self.market, state, event, context)
        } else {
            EventReducer::reduce(&self.account, state, event, context)
        }
    }

    fn validate_block(
        &self,
        state: &StateView<'_>,
        context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        EventReducer::validate_block(&self.market, state, context)?;
        EventReducer::validate_block(&self.account, state, context)
    }
}

#[derive(Debug, Clone)]
struct InjectionDispatcher {
    injections: Vec<StateMutation>,
    market: CanonicalMarketReducerV1,
    account: CanonicalAccountReducerV1,
}

impl EventReducer for InjectionDispatcher {
    fn reducer_set_version(&self) -> &str {
        "account-collision-test@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.event_kind() == EventKind::LiquidationStarted
            || EventReducer::supports(&self.market, event)
            || EventReducer::supports(&self.account, event)
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        if event.event_kind() == EventKind::LiquidationStarted {
            Ok(self.injections.clone())
        } else if EventReducer::supports(&self.market, event) {
            EventReducer::reduce(&self.market, state, event, context)
        } else {
            EventReducer::reduce(&self.account, state, event, context)
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct OversizedKeyReducer;

impl EventReducer for OversizedKeyReducer {
    fn reducer_set_version(&self) -> &str {
        "account-key-ceiling-test@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.event_kind() == EventKind::AccountModeChanged
    }

    fn reduce(
        &self,
        _state: &StateView<'_>,
        _event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        let key = StateKey::try_new("account-test-oversized.v1", vec![1; 4 * 1024 + 1]).unwrap();
        Ok(vec![StateMutation::put(key, vec![1])])
    }
}

#[test]
fn owns_exactly_nineteen_kinds_at_exact_schema() {
    let reducer = CanonicalAccountReducerV1;
    for payload in all_owned_payloads() {
        let exact = event_for(100, 0, payload.clone(), "1.0.0");
        assert!(EventReducer::supports(&reducer, &exact));
        let later = event_for(100, 0, payload, "1.1.0");
        assert!(!EventReducer::supports(&reducer, &later));
    }

    let foreign = event_for(
        100,
        0,
        EventPayload::AssetContextUpdated(AssetContextUpdated {
            asset_id: asset(),
            context_version: "v1".to_owned(),
            context_hash: [7; 32],
        }),
        "1.0.0",
    );
    assert!(!EventReducer::supports(&reducer, &foreign));
}

#[test]
fn real_market_prerequisites_enable_external_transfer_fee_reward_and_funding_flows() {
    let mut ledger = seeded_ledger(200);
    let asset = asset();
    let market = market();
    let events = vec![
        event_for(
            201,
            0,
            EventPayload::DepositCredited(DepositCredited {
                account_id: ACCOUNT_A,
                asset_id: asset.clone(),
                amount: quantity("12.50"),
                deposit_reference: "deposit-1".to_owned(),
            }),
            "1.0.0",
        ),
        event_for(
            201,
            1,
            EventPayload::WithdrawalDebited(WithdrawalDebited {
                account_id: ACCOUNT_A,
                asset_id: asset.clone(),
                amount: quantity("2.5"),
                withdrawal_reference: "withdrawal-1".to_owned(),
            }),
            "1.0.0",
        ),
        event_for(
            201,
            2,
            EventPayload::SpotTransfer(SpotTransfer {
                from_account_id: ACCOUNT_A,
                to_account_id: ACCOUNT_B,
                asset_id: asset.clone(),
                amount: quantity("1.25"),
            }),
            "1.0.0",
        ),
        event_for(
            201,
            3,
            EventPayload::FeeCharged(FeeCharged {
                account_id: ACCOUNT_A,
                asset_id: asset.clone(),
                amount: quantity("0.10"),
                fee_rate: FeeRate::from_str("0.001").unwrap(),
                fee_type: FeeTypeV1::Taker,
            }),
            "1.0.0",
        ),
        event_for(
            201,
            4,
            EventPayload::FeeCharged(FeeCharged {
                account_id: ACCOUNT_A,
                asset_id: asset.clone(),
                amount: quantity("0.05"),
                fee_rate: FeeRate::from_str("-0.0001").unwrap(),
                fee_type: FeeTypeV1::MakerRebate,
            }),
            "1.0.0",
        ),
        event_for(
            201,
            5,
            EventPayload::BuilderFeeCharged(BuilderFeeCharged {
                account_id: ACCOUNT_A,
                builder_account_id: BUILDER,
                asset_id: asset.clone(),
                amount: quantity("0.02"),
            }),
            "1.0.0",
        ),
        event_for(
            201,
            6,
            EventPayload::ReferralReward(ReferralReward {
                account_id: ACCOUNT_A,
                referrer_account_id: REFERRER,
                asset_id: asset.clone(),
                amount: quantity("0.03"),
            }),
            "1.0.0",
        ),
        event_for(
            201,
            7,
            EventPayload::FundingPaid(FundingPaid {
                account_id: ACCOUNT_A,
                market_id: market.clone(),
                amount: quote("4.0"),
                funding_rate: FundingRate::from_str("-0.001").unwrap(),
            }),
            "1.0.0",
        ),
        event_for(
            201,
            8,
            EventPayload::FundingReceived(FundingReceived {
                account_id: ACCOUNT_A,
                market_id: market.clone(),
                amount: quote("1.50"),
                funding_rate: FundingRate::from_str("0.001").unwrap(),
            }),
            "1.0.0",
        ),
    ];
    let ApplyOutcome::Applied(delta) = ledger.apply_block(&block(201, events.clone())).unwrap()
    else {
        panic!("account block must apply");
    };
    assert_eq!(
        delta
            .mutations()
            .iter()
            .filter(|mutation| mutation.key().namespace() == "account-fact.v1")
            .count(),
        events.len()
    );

    let external = quantity_flow(
        &ledger,
        ACCOUNT_A,
        AccountQuantityFlowScopeV1::ExternalAsset {
            asset_id: asset.clone(),
        },
    );
    assert_eq!(external.credits(), quantity("12.50"));
    assert_eq!(external.debits(), quantity("2.50"));

    let spot_a = quantity_flow(
        &ledger,
        ACCOUNT_A,
        AccountQuantityFlowScopeV1::SpotTransferAsset {
            asset_id: asset.clone(),
        },
    );
    let spot_b = quantity_flow(
        &ledger,
        ACCOUNT_B,
        AccountQuantityFlowScopeV1::SpotTransferAsset {
            asset_id: asset.clone(),
        },
    );
    assert_eq!(spot_a.debits(), spot_b.credits());

    let fee = quantity_flow(
        &ledger,
        ACCOUNT_A,
        AccountQuantityFlowScopeV1::FeeAsset {
            asset_id: asset.clone(),
        },
    );
    assert_eq!(fee.debits(), quantity("0.10"));
    assert_eq!(fee.credits(), quantity("0.05"));

    let builder_debit = quantity_flow(
        &ledger,
        ACCOUNT_A,
        AccountQuantityFlowScopeV1::BuilderFeeAsset {
            asset_id: asset.clone(),
        },
    );
    let builder_credit = quantity_flow(
        &ledger,
        BUILDER,
        AccountQuantityFlowScopeV1::BuilderFeeAsset {
            asset_id: asset.clone(),
        },
    );
    assert_eq!(builder_debit.debits(), builder_credit.credits());

    let referral = quantity_flow(
        &ledger,
        REFERRER,
        AccountQuantityFlowScopeV1::ReferralRewardAsset { asset_id: asset },
    );
    assert_eq!(referral.credits(), quantity("0.03"));

    let funding = quote_flow(
        &ledger,
        ACCOUNT_A,
        AccountQuoteFlowScopeV1::MarketFunding { market_id: market },
    );
    assert_eq!(funding.debits(), quote("4.00"));
    assert_eq!(funding.credits(), quote("1.50"));
}

#[test]
fn perp_subaccount_and_vault_legs_are_atomic_and_unit_separated() {
    let mut ledger = seeded_ledger(300);
    let asset = asset();
    let vault = VaultId::new("vault-alpha").unwrap();
    let events = vec![
        event_for(
            301,
            0,
            EventPayload::PerpTransfer(PerpTransfer {
                from_account_id: ACCOUNT_A,
                to_account_id: ACCOUNT_B,
                quote_amount: quote("9.50"),
            }),
            "1.0.0",
        ),
        event_for(
            301,
            1,
            EventPayload::SubaccountTransfer(SubaccountTransfer {
                master_account_id: ACCOUNT_A,
                from_account_id: ACCOUNT_A,
                to_account_id: ACCOUNT_C,
                asset_id: asset.clone(),
                amount: quantity("3.0"),
            }),
            "1.0.0",
        ),
        event_for(
            301,
            2,
            EventPayload::VaultDeposit(VaultDeposit {
                vault_id: vault.clone(),
                account_id: ACCOUNT_A,
                amount: quote("100.00"),
                shares_issued: quantity("7.5"),
            }),
            "1.0.0",
        ),
        event_for(
            301,
            3,
            EventPayload::VaultWithdrawal(VaultWithdrawal {
                vault_id: vault.clone(),
                account_id: ACCOUNT_A,
                amount: quote("20.0"),
                shares_redeemed: quantity("1.25"),
            }),
            "1.0.0",
        ),
    ];
    ledger.apply_block(&block(301, events)).unwrap();

    let perp_a = quote_flow(
        &ledger,
        ACCOUNT_A,
        AccountQuoteFlowScopeV1::DefaultPerpQuote,
    );
    let perp_b = quote_flow(
        &ledger,
        ACCOUNT_B,
        AccountQuoteFlowScopeV1::DefaultPerpQuote,
    );
    assert_eq!(perp_a.debits(), perp_b.credits());

    let relation_key = SubaccountMasterCurrentRecordV1::state_key(&ACCOUNT_C).unwrap();
    let relation = SubaccountMasterCurrentRecordV1::decode_at(
        &relation_key,
        ledger.state_image().entries().get(&relation_key).unwrap(),
    )
    .unwrap();
    assert_eq!(relation.master_account_id(), ACCOUNT_A);

    let principal = quote_flow(
        &ledger,
        ACCOUNT_A,
        AccountQuoteFlowScopeV1::VaultPrincipal {
            vault_id: vault.clone(),
        },
    );
    assert_eq!(principal.debits(), quote("100.00"));
    assert_eq!(principal.credits(), quote("20.00"));
    let shares = quantity_flow(
        &ledger,
        ACCOUNT_A,
        AccountQuantityFlowScopeV1::VaultShares {
            vault_id: vault.clone(),
        },
    );
    assert_eq!(shares.credits(), quantity("7.50"));
    assert_eq!(shares.debits(), quantity("1.25"));
    let vault_principal_key = VaultPrincipalFlowCurrentRecordV1::state_key(&vault).unwrap();
    let vault_principal = VaultPrincipalFlowCurrentRecordV1::decode_at(
        &vault_principal_key,
        ledger
            .state_image()
            .entries()
            .get(&vault_principal_key)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(vault_principal.deposits(), principal.debits());
    assert_eq!(vault_principal.withdrawals(), principal.credits());
    let vault_share_key = VaultShareFlowCurrentRecordV1::state_key(&vault).unwrap();
    let vault_share = VaultShareFlowCurrentRecordV1::decode_at(
        &vault_share_key,
        ledger
            .state_image()
            .entries()
            .get(&vault_share_key)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(vault_share.shares_issued(), shares.credits());
    assert_eq!(vault_share.shares_redeemed(), shares.debits());
}

#[test]
fn current_modes_bind_predecessors_and_allow_cycles() {
    let mut ledger = seeded_ledger(400);
    let market = market();
    ledger
        .apply_block(&block(
            401,
            vec![
                event_for(
                    401,
                    0,
                    account_mode(
                        AccountAbstractionModeV1::Standard,
                        AccountAbstractionModeV1::Unified,
                    ),
                    "1.0.0",
                ),
                event_for(
                    401,
                    1,
                    account_mode(
                        AccountAbstractionModeV1::Unified,
                        AccountAbstractionModeV1::Standard,
                    ),
                    "1.0.0",
                ),
                event_for(
                    401,
                    2,
                    margin_mode(market.clone(), MarginModeV1::Cross, MarginModeV1::Isolated),
                    "1.0.0",
                ),
                event_for(401, 3, leverage(market.clone(), "3", "5"), "1.0.0"),
            ],
        ))
        .unwrap();

    let account_key = AccountModeCurrentRecordV1::state_key(&ACCOUNT_A).unwrap();
    let account = AccountModeCurrentRecordV1::decode_at(
        &account_key,
        ledger.state_image().entries().get(&account_key).unwrap(),
    )
    .unwrap();
    assert_eq!(
        account.initial_previous(),
        AccountAbstractionModeV1::Standard
    );
    assert_eq!(account.current(), AccountAbstractionModeV1::Standard);

    let margin_key = MarginModeCurrentRecordV1::state_key(&ACCOUNT_A, &market).unwrap();
    assert!(
        MarginModeCurrentRecordV1::decode_at(
            &margin_key,
            ledger.state_image().entries().get(&margin_key).unwrap()
        )
        .is_ok()
    );
    let leverage_key = LeverageCurrentRecordV1::state_key(&ACCOUNT_A, &market).unwrap();
    assert!(
        LeverageCurrentRecordV1::decode_at(
            &leverage_key,
            ledger.state_image().entries().get(&leverage_key).unwrap()
        )
        .is_ok()
    );
}

#[test]
fn identity_mismatch_missing_prerequisite_and_late_failure_roll_back() {
    let mut ledger = bare_ledger(500);
    let missing_asset = event_for(
        500,
        0,
        EventPayload::DepositCredited(DepositCredited {
            account_id: ACCOUNT_A,
            asset_id: asset(),
            amount: quantity("1"),
            deposit_reference: "deposit".to_owned(),
        }),
        "1.0.0",
    );
    assert_reducer_reason(
        ledger.apply_block(&block(500, vec![missing_asset])),
        "account_state.asset_prerequisite_missing",
    );

    let mut ledger = seeded_ledger(510);
    let valid = event_for(
        511,
        0,
        EventPayload::DepositCredited(DepositCredited {
            account_id: ACCOUNT_A,
            asset_id: asset(),
            amount: quantity("1"),
            deposit_reference: "deposit".to_owned(),
        }),
        "1.0.0",
    );
    let mut invalid = event_for(
        511,
        1,
        EventPayload::SpotTransfer(SpotTransfer {
            from_account_id: ACCOUNT_A,
            to_account_id: ACCOUNT_B,
            asset_id: asset(),
            amount: quantity("1"),
        }),
        "1.0.0",
    );
    invalid = rebuild_with_identities(invalid, vec![ACCOUNT_B, ACCOUNT_A], Vec::new());
    let before = ledger.state_image().canonical_bytes();
    assert_reducer_reason(
        ledger.apply_block(&block(511, vec![valid, invalid])),
        "account_state.identity_mismatch",
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn ordered_identity_shapes_reject_missing_extra_reordered_and_deduplicated_values() {
    let asset = asset();
    let exact = event_for(
        801,
        0,
        EventPayload::SubaccountTransfer(SubaccountTransfer {
            master_account_id: ACCOUNT_A,
            from_account_id: ACCOUNT_A,
            to_account_id: ACCOUNT_C,
            asset_id: asset,
            amount: quantity("1"),
        }),
        "1.0.0",
    );
    let variants = [
        (vec![ACCOUNT_A, ACCOUNT_A], Vec::new()),
        (vec![ACCOUNT_A, ACCOUNT_A, ACCOUNT_C, ACCOUNT_B], Vec::new()),
        (vec![ACCOUNT_A, ACCOUNT_C, ACCOUNT_A], Vec::new()),
        (vec![ACCOUNT_A, ACCOUNT_C], Vec::new()),
        (vec![ACCOUNT_A, ACCOUNT_A, ACCOUNT_C], vec![market()]),
    ];
    for (accounts, markets) in variants {
        let mut ledger = seeded_ledger(800);
        let invalid = rebuild_with_identities(exact.clone(), accounts, markets);
        let before = ledger.state_image().canonical_bytes();
        assert_reducer_reason(
            ledger.apply_block(&block(801, vec![invalid])),
            "account_state.identity_mismatch",
        );
        assert_eq!(ledger.state_image().canonical_bytes(), before);
    }
}

#[test]
fn all_non_rebate_fee_types_debit_and_referral_never_debits_primary_account() {
    let mut ledger = seeded_ledger(820);
    let asset = asset();
    let mut events = Vec::new();
    for (index, fee_type) in [
        FeeTypeV1::Maker,
        FeeTypeV1::Taker,
        FeeTypeV1::ReferralDiscount,
        FeeTypeV1::Protocol,
    ]
    .into_iter()
    .enumerate()
    {
        events.push(event_for(
            821,
            u32::try_from(index).unwrap(),
            EventPayload::FeeCharged(FeeCharged {
                account_id: ACCOUNT_A,
                asset_id: asset.clone(),
                amount: quantity("0.1"),
                fee_rate: FeeRate::from_str("0.001").unwrap(),
                fee_type,
            }),
            "1.0.0",
        ));
    }
    events.push(event_for(
        821,
        4,
        EventPayload::ReferralReward(ReferralReward {
            account_id: ACCOUNT_A,
            referrer_account_id: REFERRER,
            asset_id: asset.clone(),
            amount: quantity("0.2"),
        }),
        "1.0.0",
    ));
    ledger.apply_block(&block(821, events)).unwrap();
    let fees = quantity_flow(
        &ledger,
        ACCOUNT_A,
        AccountQuantityFlowScopeV1::FeeAsset {
            asset_id: asset.clone(),
        },
    );
    assert_eq!(fees.debits(), quantity("0.4"));
    let primary_reward_key = AccountQuantityFlowCurrentRecordV1::state_key(
        &ACCOUNT_A,
        &AccountQuantityFlowScopeV1::ReferralRewardAsset {
            asset_id: asset.clone(),
        },
    )
    .unwrap();
    assert!(
        !ledger
            .state_image()
            .entries()
            .contains_key(&primary_reward_key)
    );
    let referrer = quantity_flow(
        &ledger,
        REFERRER,
        AccountQuantityFlowScopeV1::ReferralRewardAsset { asset_id: asset },
    );
    assert_eq!(referrer.credits(), quantity("0.2"));
    assert_eq!(referrer.debits(), quantity("0.0"));
}

#[test]
fn every_mode_family_rejects_mismatched_predecessors_and_accepts_cycles() {
    let mut ledger = seeded_ledger(840);
    let market = market();
    ledger
        .apply_block(&block(
            841,
            vec![
                event_for(
                    841,
                    0,
                    account_mode(
                        AccountAbstractionModeV1::Standard,
                        AccountAbstractionModeV1::Unified,
                    ),
                    "1.0.0",
                ),
                event_for(
                    841,
                    1,
                    margin_mode(market.clone(), MarginModeV1::Cross, MarginModeV1::Isolated),
                    "1.0.0",
                ),
                event_for(841, 2, leverage(market.clone(), "3", "5"), "1.0.0"),
            ],
        ))
        .unwrap();

    let cases = [
        (
            account_mode(
                AccountAbstractionModeV1::Portfolio,
                AccountAbstractionModeV1::Standard,
            ),
            "account_state.previous_account_mode_mismatch",
        ),
        (
            margin_mode(
                market.clone(),
                MarginModeV1::StrictIsolated,
                MarginModeV1::Cross,
            ),
            "account_state.previous_margin_mode_mismatch",
        ),
        (
            leverage(market.clone(), "4", "3"),
            "account_state.previous_leverage_mismatch",
        ),
    ];
    for (payload, reason) in cases {
        let height = 842;
        let before = ledger.state_image().canonical_bytes();
        assert_reducer_reason(
            ledger.apply_block(&block(height, vec![event_for(height, 0, payload, "1.0.0")])),
            reason,
        );
        assert_eq!(ledger.state_image().canonical_bytes(), before);
    }

    ledger
        .apply_block(&block(
            842,
            vec![
                event_for(
                    842,
                    0,
                    account_mode(
                        AccountAbstractionModeV1::Unified,
                        AccountAbstractionModeV1::Standard,
                    ),
                    "1.0.0",
                ),
                event_for(
                    842,
                    1,
                    margin_mode(market.clone(), MarginModeV1::Isolated, MarginModeV1::Cross),
                    "1.0.0",
                ),
                event_for(842, 2, leverage(market, "5", "3"), "1.0.0"),
            ],
        ))
        .unwrap();
}

#[test]
fn ambiguous_and_conflicting_subaccount_relations_fail_closed() {
    let mut ledger = seeded_ledger(600);
    let ambiguous = event_for(
        601,
        0,
        EventPayload::SubaccountTransfer(SubaccountTransfer {
            master_account_id: ACCOUNT_A,
            from_account_id: ACCOUNT_B,
            to_account_id: ACCOUNT_C,
            asset_id: asset(),
            amount: quantity("1"),
        }),
        "1.0.0",
    );
    assert_reducer_reason(
        ledger.apply_block(&block(601, vec![ambiguous])),
        "account_state.subaccount_scope_ambiguous",
    );

    let establish = event_for(
        601,
        0,
        EventPayload::SubaccountTransfer(SubaccountTransfer {
            master_account_id: ACCOUNT_A,
            from_account_id: ACCOUNT_A,
            to_account_id: ACCOUNT_C,
            asset_id: asset(),
            amount: quantity("1"),
        }),
        "1.0.0",
    );
    ledger.apply_block(&block(601, vec![establish])).unwrap();
    let conflict = event_for(
        602,
        0,
        EventPayload::SubaccountTransfer(SubaccountTransfer {
            master_account_id: ACCOUNT_B,
            from_account_id: ACCOUNT_B,
            to_account_id: ACCOUNT_C,
            asset_id: asset(),
            amount: quantity("1"),
        }),
        "1.0.0",
    );
    let before = ledger.state_image().canonical_bytes();
    assert_reducer_reason(
        ledger.apply_block(&block(602, vec![conflict])),
        "account_state.master_conflict",
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn master_as_to_establishes_one_relation_and_same_master_refresh_preserves_first_observation() {
    let mut ledger = seeded_ledger(620);
    let first = event_for(
        621,
        0,
        EventPayload::SubaccountTransfer(SubaccountTransfer {
            master_account_id: ACCOUNT_A,
            from_account_id: ACCOUNT_C,
            to_account_id: ACCOUNT_A,
            asset_id: asset(),
            amount: quantity("1"),
        }),
        "1.0.0",
    );
    let first_event_id = first.event_id().clone();
    ledger.apply_block(&block(621, vec![first])).unwrap();
    let refresh = event_for(
        622,
        0,
        EventPayload::SubaccountTransfer(SubaccountTransfer {
            master_account_id: ACCOUNT_A,
            from_account_id: ACCOUNT_A,
            to_account_id: ACCOUNT_C,
            asset_id: asset(),
            amount: quantity("2"),
        }),
        "1.0.0",
    );
    let last_event_id = refresh.event_id().clone();
    ledger.apply_block(&block(622, vec![refresh])).unwrap();

    let key = SubaccountMasterCurrentRecordV1::state_key(&ACCOUNT_C).unwrap();
    let relation = SubaccountMasterCurrentRecordV1::decode_at(
        &key,
        ledger.state_image().entries().get(&key).unwrap(),
    )
    .unwrap();
    assert_eq!(relation.master_account_id(), ACCOUNT_A);
    assert_eq!(relation.first_event_id(), &first_event_id);
    assert_eq!(relation.last_event_id(), &last_event_id);
    assert_eq!(relation.first_block_height(), BlockHeight::new(621));
    assert_eq!(relation.last_block_height(), BlockHeight::new(622));
    assert_eq!(
        ledger
            .state_image()
            .entries()
            .keys()
            .filter(|state_key| state_key.namespace() == "account-subaccount-master.v1")
            .count(),
        1
    );
}

#[test]
fn market_prerequisites_distinguish_missing_from_unresolved_metadata() {
    let market = market();
    let missing = event_for(
        870,
        0,
        EventPayload::FundingPaid(FundingPaid {
            account_id: ACCOUNT_A,
            market_id: market.clone(),
            amount: quote("1"),
            funding_rate: FundingRate::from_str("0.001").unwrap(),
        }),
        "1.0.0",
    );
    let mut bare = bare_ledger(870);
    assert_reducer_reason(
        bare.apply_block(&block(870, vec![missing])),
        "account_state.market_prerequisite_missing",
    );

    let mut unresolved = seeded_ledger(880);
    unresolved
        .apply_block(&block(
            881,
            vec![raw_event(
                881,
                0,
                EventPayload::MarketMetadataChanged(MarketMetadataChanged {
                    market_id: market.clone(),
                    metadata_version: "metadata-v2".to_owned(),
                    metadata_hash: [9; 32],
                }),
                vec![market.clone()],
                Vec::new(),
                "1.0.0",
            )],
        ))
        .unwrap();
    let funding = event_for(
        882,
        0,
        EventPayload::FundingReceived(FundingReceived {
            account_id: ACCOUNT_A,
            market_id: market,
            amount: quote("1"),
            funding_rate: FundingRate::from_str("-0.001").unwrap(),
        }),
        "1.0.0",
    );
    let before = unresolved.state_image().canonical_bytes();
    assert_reducer_reason(
        unresolved.apply_block(&block(882, vec![funding])),
        "account_state.market_metadata_unresolved",
    );
    assert_eq!(unresolved.state_image().canonical_bytes(), before);
}

#[test]
fn corrupt_and_key_mismatched_asset_and_market_prerequisites_fail_closed() {
    let asset_key = AssetContextCurrentRecordV1::state_key(&asset()).unwrap();
    let valid_wrong_asset = br#"{"schema":"hyperliquid-alpha-desk/asset-context-current/v1","asset_id":"BTC","context_version":"btc-v1","context_blake3":"0101010101010101010101010101010101010101010101010101010101010101","updated_at_block":900}"#.to_vec();
    for (offset, value) in [b"corrupt".to_vec(), valid_wrong_asset]
        .into_iter()
        .enumerate()
    {
        let height = 900 + u64::try_from(offset).unwrap() * 10;
        let mut ledger = injected_ledger(
            height,
            vec![StateMutation::put(asset_key.clone(), value)],
            Vec::new(),
        );
        let deposit = event_for(
            height + 1,
            0,
            EventPayload::DepositCredited(DepositCredited {
                account_id: ACCOUNT_A,
                asset_id: asset(),
                amount: quantity("1"),
                deposit_reference: "deposit".to_owned(),
            }),
            "1.0.0",
        );
        let before = ledger.state_image().canonical_bytes();
        assert_reducer_reason(
            ledger.apply_block(&block(height + 1, vec![deposit])),
            "account_state.asset_prerequisite_invalid",
        );
        assert_eq!(ledger.state_image().canonical_bytes(), before);
    }

    let market_key = MarketCurrentRecordV1::state_key(&market()).unwrap();
    let valid_wrong_market = br#"{"schema":"hyperliquid-alpha-desk/market-current/v1","market_id":"perp:ETH","dex_id":"validator","base_asset_id":"ETH","quote_asset_id":"USDC","status":"active","metadata_resolution":"exact","metadata_version":"creation@1.0.0","metadata_blake3":"0202020202020202020202020202020202020202020202020202020202020202","tick_size":"0.100000","lot_size":"0.00100000","price_scale":6,"quantity_scale":8,"open_interest_cap":null,"margin_table_hash":null,"oracle_price":null,"oracle_source":null,"oracle_effective_at_micros":null,"funding_rate":null,"funding_effective_at_micros":null,"created_at_block":920,"updated_at_block":920}"#.to_vec();
    for (offset, value) in [b"corrupt".to_vec(), valid_wrong_market]
        .into_iter()
        .enumerate()
    {
        let height = 920 + u64::try_from(offset).unwrap() * 10;
        let mut ledger = injected_ledger(
            height,
            vec![StateMutation::put(market_key.clone(), value)],
            Vec::new(),
        );
        let funding = event_for(
            height + 1,
            0,
            EventPayload::FundingPaid(FundingPaid {
                account_id: ACCOUNT_A,
                market_id: market(),
                amount: quote("1"),
                funding_rate: FundingRate::from_str("0.001").unwrap(),
            }),
            "1.0.0",
        );
        let before = ledger.state_image().canonical_bytes();
        assert_reducer_reason(
            ledger.apply_block(&block(height + 1, vec![funding])),
            "account_state.market_prerequisite_invalid",
        );
        assert_eq!(ledger.state_image().canonical_bytes(), before);
    }
}

#[test]
fn corrupt_current_record_and_exact_scale_or_addition_overflow_never_replace_state() {
    let scope = AccountQuantityFlowScopeV1::ExternalAsset { asset_id: asset() };
    let current_key = AccountQuantityFlowCurrentRecordV1::state_key(&ACCOUNT_A, &scope).unwrap();
    let asset_event = raw_event(
        950,
        0,
        EventPayload::AssetContextUpdated(AssetContextUpdated {
            asset_id: asset(),
            context_version: "usdc-v1".to_owned(),
            context_hash: [2; 32],
        }),
        Vec::new(),
        Vec::new(),
        "1.0.0",
    );
    let mut corrupt = injected_ledger(
        950,
        vec![StateMutation::put(current_key.clone(), b"corrupt".to_vec())],
        vec![asset_event],
    );
    let deposit = event_for(
        951,
        0,
        EventPayload::DepositCredited(DepositCredited {
            account_id: ACCOUNT_A,
            asset_id: asset(),
            amount: quantity("1"),
            deposit_reference: "deposit".to_owned(),
        }),
        "1.0.0",
    );
    let before = corrupt.state_image().canonical_bytes();
    assert_reducer_reason(
        corrupt.apply_block(&block(951, vec![deposit])),
        "account_state.current_record_invalid",
    );
    assert_eq!(corrupt.state_image().canonical_bytes(), before);

    for (offset, amount) in ["1", "0.1"].into_iter().enumerate() {
        let height = 960 + u64::try_from(offset).unwrap() * 10;
        let record = format!(
            "{{\"schema\":\"hyperliquid-alpha-desk/account-quantity-flow-current/v1\",\"account_id\":\"{}\",\"scope\":\"external_asset\",\"asset_id\":\"USDC\",\"vault_id\":null,\"credits\":\"{}\",\"debits\":\"0\",\"last_event_id\":\"event-seed\",\"last_block_height\":{height}}}",
            ACCOUNT_A.to_api_string(),
            i128::MAX
        )
        .into_bytes();
        let prerequisite = raw_event(
            height,
            0,
            EventPayload::AssetContextUpdated(AssetContextUpdated {
                asset_id: asset(),
                context_version: "usdc-v1".to_owned(),
                context_hash: [2; 32],
            }),
            Vec::new(),
            Vec::new(),
            "1.0.0",
        );
        let mut ledger = injected_ledger(
            height,
            vec![StateMutation::put(current_key.clone(), record)],
            vec![prerequisite],
        );
        let overflow = event_for(
            height + 1,
            0,
            EventPayload::DepositCredited(DepositCredited {
                account_id: ACCOUNT_A,
                asset_id: asset(),
                amount: quantity(amount),
                deposit_reference: "deposit".to_owned(),
            }),
            "1.0.0",
        );
        let before = ledger.state_image().canonical_bytes();
        assert_reducer_reason(
            ledger.apply_block(&block(height + 1, vec![overflow])),
            "account_state.flow_arithmetic",
        );
        assert_eq!(ledger.state_image().canonical_bytes(), before);
    }
}

#[test]
fn production_ledger_rejects_a_test_only_mutation_above_the_four_kib_key_ceiling() {
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(990),
        OversizedKeyReducer,
        LedgerLimits::production(),
    )
    .unwrap();
    let event = event_for(
        990,
        0,
        account_mode(
            AccountAbstractionModeV1::Standard,
            AccountAbstractionModeV1::Unified,
        ),
        "1.0.0",
    );
    let before = ledger.state_image().canonical_bytes();
    let error = ledger.apply_block(&block(990, vec![event])).unwrap_err();
    assert_eq!(error.reason_code(), "ledger.mutation_limit_exceeded");
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn complete_owned_state_restores_byte_exactly_with_frozen_namespace_counts() {
    let mut ledger = seeded_ledger(1_000);
    let events = all_owned_payloads()
        .into_iter()
        .enumerate()
        .map(|(index, payload)| event_for(1_001, u32::try_from(index).unwrap(), payload, "1.0.0"))
        .collect::<Vec<_>>();
    ledger.apply_block(&block(1_001, events)).unwrap();
    let entries = ledger.state_image().entries();
    let expected = [
        ("account-fact.v1", 19),
        ("account-quantity-flow-current.v1", 11),
        ("account-quote-flow-current.v1", 6),
        ("vault-principal-flow-current.v1", 1),
        ("vault-share-flow-current.v1", 1),
        ("account-subaccount-master.v1", 1),
        ("account-vault-relation.v1", 1),
        ("account-mode-current.v1", 1),
        ("account-margin-mode-current.v1", 1),
        ("account-leverage-current.v1", 1),
    ];
    for (namespace, count) in expected {
        assert_eq!(
            entries
                .keys()
                .filter(|key| key.namespace() == namespace)
                .count(),
            count,
            "unexpected count for {namespace}"
        );
    }

    let bytes = ledger.state_image().canonical_bytes();
    let restored = StateImage::decode_canonical(&bytes, StateImageLimits::production()).unwrap();
    assert_eq!(restored.canonical_bytes(), bytes);
    assert_eq!(restored.state_hash(), ledger.state_hash());
    for (key, value) in restored
        .entries()
        .iter()
        .filter(|(key, _)| key.namespace() == "account-fact.v1")
    {
        assert!(AccountFactRecordV1::decode_at(key, value).is_ok());
    }
}

#[test]
fn immutable_event_fact_collision_is_rejected_at_later_height() {
    let colliding = event_for(
        701,
        0,
        EventPayload::DepositCredited(DepositCredited {
            account_id: ACCOUNT_A,
            asset_id: asset(),
            amount: quantity("1"),
            deposit_reference: "deposit".to_owned(),
        }),
        "1.0.0",
    );
    let target = AccountFactRecordV1::state_key(colliding.event_id()).unwrap();
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(700),
        InjectionDispatcher {
            injections: vec![StateMutation::put(target, vec![1])],
            market: CanonicalMarketReducerV1,
            account: CanonicalAccountReducerV1,
        },
        LedgerLimits::production(),
    )
    .unwrap();
    ledger
        .apply_block(&block(
            700,
            vec![
                raw_event(
                    700,
                    0,
                    EventPayload::AssetContextUpdated(AssetContextUpdated {
                        asset_id: asset(),
                        context_version: "usdc-v1".to_owned(),
                        context_hash: [2; 32],
                    }),
                    Vec::new(),
                    Vec::new(),
                    "1.0.0",
                ),
                raw_event(
                    700,
                    1,
                    EventPayload::LiquidationStarted(LiquidationStarted {
                        account_id: ACCOUNT_A,
                        liquidation_id: LiquidationId::new("liq-seed").unwrap(),
                        margin_value: UsdAmount::from_str("1").unwrap(),
                        maintenance_requirement: UsdAmount::from_str("2").unwrap(),
                    }),
                    Vec::new(),
                    vec![ACCOUNT_A],
                    "1.0.0",
                ),
            ],
        ))
        .unwrap();
    let before = ledger.state_image().canonical_bytes();
    assert_reducer_reason(
        ledger.apply_block(&block(701, vec![colliding])),
        "account_state.event_identity_collision",
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn internal_transfer_debits_amount_plus_fee_once_and_credits_destination_amount() {
    let mut ledger = seeded_ledger(800);
    ledger
        .apply_block(&block(
            801,
            vec![raw_event(
                801,
                0,
                EventPayload::decode(
                    EventKind::InternalTransfer,
                    &encode_internal_transfer(&WireInternalTransfer {
                        from_account_id: ACCOUNT_A.to_api_string(),
                        to_account_id: ACCOUNT_B.to_api_string(),
                        amount: "1.00".to_owned(),
                        fee: "0.01".to_owned(),
                    })
                    .unwrap(),
                )
                .unwrap(),
                Vec::new(),
                vec![ACCOUNT_A, ACCOUNT_B],
                "1.0.0",
            )],
        ))
        .unwrap();
    let source = quote_flow(
        &ledger,
        ACCOUNT_A,
        AccountQuoteFlowScopeV1::DefaultPerpQuote,
    );
    let destination = quote_flow(
        &ledger,
        ACCOUNT_B,
        AccountQuoteFlowScopeV1::DefaultPerpQuote,
    );
    assert_eq!(source.debits(), quote("1.01"));
    assert_eq!(destination.credits(), quote("1.00"));
}

#[test]
fn account_class_transfer_moves_quote_between_spot_and_perp_scopes() {
    let mut ledger = seeded_ledger(810);
    ledger
        .apply_block(&block(811, vec![class_transfer(811, 0, true, "1.00")]))
        .unwrap();
    assert_eq!(
        quote_flow(&ledger, ACCOUNT_A, AccountQuoteFlowScopeV1::SpotClassQuote).debits(),
        quote("1.00")
    );
    assert_eq!(
        quote_flow(
            &ledger,
            ACCOUNT_A,
            AccountQuoteFlowScopeV1::DefaultPerpQuote
        )
        .credits(),
        quote("1.00")
    );
    ledger
        .apply_block(&block(812, vec![class_transfer(812, 0, false, "0.40")]))
        .unwrap();
    assert_eq!(
        quote_flow(&ledger, ACCOUNT_A, AccountQuoteFlowScopeV1::SpotClassQuote).credits(),
        quote("0.40")
    );
}

#[test]
fn reward_claimed_credits_dedicated_quote_scope() {
    let mut ledger = seeded_ledger(820);
    ledger
        .apply_block(&block(
            821,
            vec![raw_event(
                821,
                0,
                EventPayload::decode(
                    EventKind::RewardClaimed,
                    &encode_reward_claimed(&WireRewardClaimed {
                        account_id: ACCOUNT_A.to_api_string(),
                        amount: "1.00".to_owned(),
                    })
                    .unwrap(),
                )
                .unwrap(),
                Vec::new(),
                vec![ACCOUNT_A],
                "1.0.0",
            )],
        ))
        .unwrap();
    assert_eq!(
        quote_flow(
            &ledger,
            ACCOUNT_A,
            AccountQuoteFlowScopeV1::RewardClaimedQuote
        )
        .credits(),
        quote("1.00")
    );
}

#[test]
fn spot_genesis_credits_one_user_and_allows_token_only_zero_user() {
    let mut ledger = seeded_ledger(830);
    ledger
        .apply_block(&block(
            831,
            vec![spot_genesis(831, 0, "1.0", vec![ACCOUNT_A])],
        ))
        .unwrap();
    assert_eq!(
        quantity_flow(
            &ledger,
            ACCOUNT_A,
            AccountQuantityFlowScopeV1::SpotGenesisAsset { asset_id: asset() }
        )
        .credits(),
        quantity("1.0")
    );

    let mut ledger = seeded_ledger(840);
    ledger
        .apply_block(&block(841, vec![spot_genesis(841, 0, "1.0", Vec::new())]))
        .unwrap();
    let key = AccountQuantityFlowCurrentRecordV1::state_key(
        &ACCOUNT_A,
        &AccountQuantityFlowScopeV1::SpotGenesisAsset { asset_id: asset() },
    )
    .unwrap();
    assert!(!ledger.state_image().entries().contains_key(&key));
}

fn class_transfer(height: u64, index: u32, to_perp: bool, amount: &str) -> CanonicalEventEnvelope {
    raw_event(
        height,
        index,
        EventPayload::decode(
            EventKind::AccountClassTransfer,
            &encode_account_class_transfer(&WireAccountClassTransfer {
                account_id: ACCOUNT_A.to_api_string(),
                amount: amount.to_owned(),
                to_perp,
            })
            .unwrap(),
        )
        .unwrap(),
        Vec::new(),
        vec![ACCOUNT_A],
        "1.0.0",
    )
}

fn spot_genesis(
    height: u64,
    index: u32,
    amount: &str,
    accounts: Vec<Address>,
) -> CanonicalEventEnvelope {
    raw_event(
        height,
        index,
        EventPayload::decode(
            EventKind::SpotGenesisApplied,
            &encode_spot_genesis_applied(&WireSpotGenesisApplied {
                token: asset().as_str().to_owned(),
                amount: amount.to_owned(),
            })
            .unwrap(),
        )
        .unwrap(),
        Vec::new(),
        accounts,
        "1.0.0",
    )
}

fn all_owned_payloads() -> Vec<EventPayload> {
    let asset = asset();
    let market = market();
    let vault = VaultId::new("vault-alpha").unwrap();
    vec![
        EventPayload::DepositCredited(DepositCredited {
            account_id: ACCOUNT_A,
            asset_id: asset.clone(),
            amount: quantity("1"),
            deposit_reference: "deposit".to_owned(),
        }),
        EventPayload::WithdrawalDebited(WithdrawalDebited {
            account_id: ACCOUNT_A,
            asset_id: asset.clone(),
            amount: quantity("1"),
            withdrawal_reference: "withdrawal".to_owned(),
        }),
        EventPayload::SpotTransfer(SpotTransfer {
            from_account_id: ACCOUNT_A,
            to_account_id: ACCOUNT_B,
            asset_id: asset.clone(),
            amount: quantity("1"),
        }),
        EventPayload::PerpTransfer(PerpTransfer {
            from_account_id: ACCOUNT_A,
            to_account_id: ACCOUNT_B,
            quote_amount: quote("1"),
        }),
        EventPayload::SubaccountTransfer(SubaccountTransfer {
            master_account_id: ACCOUNT_A,
            from_account_id: ACCOUNT_A,
            to_account_id: ACCOUNT_B,
            asset_id: asset.clone(),
            amount: quantity("1"),
        }),
        EventPayload::VaultDeposit(VaultDeposit {
            vault_id: vault.clone(),
            account_id: ACCOUNT_A,
            amount: quote("1"),
            shares_issued: quantity("1"),
        }),
        EventPayload::VaultWithdrawal(VaultWithdrawal {
            vault_id: vault,
            account_id: ACCOUNT_A,
            amount: quote("1"),
            shares_redeemed: quantity("1"),
        }),
        EventPayload::FeeCharged(FeeCharged {
            account_id: ACCOUNT_A,
            asset_id: asset.clone(),
            amount: quantity("1"),
            fee_rate: FeeRate::from_str("0.001").unwrap(),
            fee_type: FeeTypeV1::Taker,
        }),
        EventPayload::BuilderFeeCharged(BuilderFeeCharged {
            account_id: ACCOUNT_A,
            builder_account_id: BUILDER,
            asset_id: asset.clone(),
            amount: quantity("1"),
        }),
        EventPayload::FundingPaid(FundingPaid {
            account_id: ACCOUNT_A,
            market_id: market.clone(),
            amount: quote("1"),
            funding_rate: FundingRate::from_str("0.001").unwrap(),
        }),
        EventPayload::FundingReceived(FundingReceived {
            account_id: ACCOUNT_A,
            market_id: market.clone(),
            amount: quote("1"),
            funding_rate: FundingRate::from_str("-0.001").unwrap(),
        }),
        EventPayload::ReferralReward(ReferralReward {
            account_id: ACCOUNT_A,
            referrer_account_id: REFERRER,
            asset_id: asset.clone(),
            amount: quantity("1"),
        }),
        EventPayload::decode(
            EventKind::InternalTransfer,
            &encode_internal_transfer(&WireInternalTransfer {
                from_account_id: ACCOUNT_A.to_api_string(),
                to_account_id: ACCOUNT_B.to_api_string(),
                amount: "1".to_owned(),
                fee: "0".to_owned(),
            })
            .unwrap(),
        )
        .unwrap(),
        EventPayload::decode(
            EventKind::AccountClassTransfer,
            &encode_account_class_transfer(&WireAccountClassTransfer {
                account_id: ACCOUNT_A.to_api_string(),
                amount: "1".to_owned(),
                to_perp: true,
            })
            .unwrap(),
        )
        .unwrap(),
        EventPayload::decode(
            EventKind::RewardClaimed,
            &encode_reward_claimed(&WireRewardClaimed {
                account_id: ACCOUNT_A.to_api_string(),
                amount: "1".to_owned(),
            })
            .unwrap(),
        )
        .unwrap(),
        EventPayload::decode(
            EventKind::SpotGenesisApplied,
            &encode_spot_genesis_applied(&WireSpotGenesisApplied {
                token: asset.as_str().to_owned(),
                amount: "1".to_owned(),
            })
            .unwrap(),
        )
        .unwrap(),
        account_mode(
            AccountAbstractionModeV1::Standard,
            AccountAbstractionModeV1::Unified,
        ),
        margin_mode(market.clone(), MarginModeV1::Cross, MarginModeV1::Isolated),
        leverage(market, "3", "5"),
    ]
}

fn account_mode(previous: AccountAbstractionModeV1, new: AccountAbstractionModeV1) -> EventPayload {
    EventPayload::AccountModeChanged(AccountModeChanged {
        account_id: ACCOUNT_A,
        previous_mode: previous,
        new_mode: new,
    })
}

fn margin_mode(market_id: MarketId, previous: MarginModeV1, new: MarginModeV1) -> EventPayload {
    EventPayload::MarginModeChanged(MarginModeChanged {
        account_id: ACCOUNT_A,
        market_id,
        previous_mode: previous,
        new_mode: new,
    })
}

fn leverage(market_id: MarketId, previous: &str, new: &str) -> EventPayload {
    EventPayload::LeverageChanged(LeverageChanged {
        account_id: ACCOUNT_A,
        market_id,
        previous_leverage: Leverage::from_str(previous).unwrap(),
        new_leverage: Leverage::from_str(new).unwrap(),
    })
}

fn seeded_ledger(first_height: u64) -> CanonicalLedger<TestDispatcher> {
    let mut ledger = bare_ledger(first_height);
    let asset = asset();
    let base = AssetId::new("BTC").unwrap();
    let market = market();
    ledger
        .apply_block(&block(
            first_height,
            vec![
                raw_event(
                    first_height,
                    0,
                    EventPayload::DexCreated(DexCreated {
                        dex_id: DexId::new("validator").unwrap(),
                        name: "Validator".to_owned(),
                        operator_account_id: OPERATOR,
                    }),
                    Vec::new(),
                    vec![OPERATOR],
                    "1.0.0",
                ),
                raw_event(
                    first_height,
                    1,
                    EventPayload::AssetContextUpdated(AssetContextUpdated {
                        asset_id: base.clone(),
                        context_version: "btc-v1".to_owned(),
                        context_hash: [1; 32],
                    }),
                    Vec::new(),
                    Vec::new(),
                    "1.0.0",
                ),
                raw_event(
                    first_height,
                    2,
                    EventPayload::AssetContextUpdated(AssetContextUpdated {
                        asset_id: asset.clone(),
                        context_version: "usdc-v1".to_owned(),
                        context_hash: [2; 32],
                    }),
                    Vec::new(),
                    Vec::new(),
                    "1.0.0",
                ),
                raw_event(
                    first_height,
                    3,
                    EventPayload::MarketCreated(MarketCreated {
                        market_id: market.clone(),
                        dex_id: DexId::new("validator").unwrap(),
                        base_asset_id: base,
                        quote_asset_id: asset,
                        tick_size: Price::parse_at_scale("0.1", 6).unwrap(),
                        lot_size: Quantity::parse_at_scale("0.001", 8).unwrap(),
                    }),
                    vec![market],
                    Vec::new(),
                    "1.0.0",
                ),
            ],
        ))
        .unwrap();
    ledger
}

fn bare_ledger(first_height: u64) -> CanonicalLedger<TestDispatcher> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        TestDispatcher::default(),
        LedgerLimits::production(),
    )
    .unwrap()
}

fn injected_ledger(
    first_height: u64,
    injections: Vec<StateMutation>,
    mut real_events: Vec<CanonicalEventEnvelope>,
) -> CanonicalLedger<InjectionDispatcher> {
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        InjectionDispatcher {
            injections,
            market: CanonicalMarketReducerV1,
            account: CanonicalAccountReducerV1,
        },
        LedgerLimits::production(),
    )
    .unwrap();
    let event_index = u32::try_from(real_events.len()).unwrap();
    real_events.push(raw_event(
        first_height,
        event_index,
        EventPayload::LiquidationStarted(LiquidationStarted {
            account_id: ACCOUNT_A,
            liquidation_id: LiquidationId::new(format!("liq-seed-{first_height}")).unwrap(),
            margin_value: UsdAmount::from_str("1").unwrap(),
            maintenance_requirement: UsdAmount::from_str("2").unwrap(),
        }),
        Vec::new(),
        vec![ACCOUNT_A],
        "1.0.0",
    ));
    ledger
        .apply_block(&block(first_height, real_events))
        .unwrap();
    ledger
}

fn quantity_flow(
    ledger: &CanonicalLedger<TestDispatcher>,
    account: Address,
    scope: AccountQuantityFlowScopeV1,
) -> AccountQuantityFlowCurrentRecordV1 {
    let key = AccountQuantityFlowCurrentRecordV1::state_key(&account, &scope).unwrap();
    AccountQuantityFlowCurrentRecordV1::decode_at(
        &key,
        ledger.state_image().entries().get(&key).unwrap(),
    )
    .unwrap()
}

fn quote_flow(
    ledger: &CanonicalLedger<TestDispatcher>,
    account: Address,
    scope: AccountQuoteFlowScopeV1,
) -> AccountQuoteFlowCurrentRecordV1 {
    let key = AccountQuoteFlowCurrentRecordV1::state_key(&account, &scope).unwrap();
    AccountQuoteFlowCurrentRecordV1::decode_at(
        &key,
        ledger.state_image().entries().get(&key).unwrap(),
    )
    .unwrap()
}

fn asset() -> AssetId {
    AssetId::new("USDC").unwrap()
}

fn market() -> MarketId {
    MarketId::new("perp:BTC").unwrap()
}

fn quantity(value: &str) -> Quantity {
    Quantity::from_str(value).unwrap()
}

fn quote(value: &str) -> QuoteAmount {
    QuoteAmount::from_str(value).unwrap()
}

fn event_for(
    height: u64,
    event_index: u32,
    payload: EventPayload,
    schema: &str,
) -> CanonicalEventEnvelope {
    let (accounts, markets) = identities(&payload);
    raw_event(height, event_index, payload, markets, accounts, schema)
}

fn identities(payload: &EventPayload) -> (Vec<Address>, Vec<MarketId>) {
    match payload {
        EventPayload::DepositCredited(value) => (vec![value.account_id], Vec::new()),
        EventPayload::WithdrawalDebited(value) => (vec![value.account_id], Vec::new()),
        EventPayload::SpotTransfer(value) => {
            (vec![value.from_account_id, value.to_account_id], Vec::new())
        }
        EventPayload::PerpTransfer(value) => {
            (vec![value.from_account_id, value.to_account_id], Vec::new())
        }
        EventPayload::SubaccountTransfer(value) => (
            vec![
                value.master_account_id,
                value.from_account_id,
                value.to_account_id,
            ],
            Vec::new(),
        ),
        EventPayload::VaultDeposit(value) => (vec![value.account_id], Vec::new()),
        EventPayload::VaultWithdrawal(value) => (vec![value.account_id], Vec::new()),
        EventPayload::FeeCharged(value) => (vec![value.account_id], Vec::new()),
        EventPayload::BuilderFeeCharged(value) => {
            (vec![value.account_id, value.builder_account_id], Vec::new())
        }
        EventPayload::FundingPaid(value) => (vec![value.account_id], vec![value.market_id.clone()]),
        EventPayload::FundingReceived(value) => {
            (vec![value.account_id], vec![value.market_id.clone()])
        }
        EventPayload::ReferralReward(value) => (
            vec![value.account_id, value.referrer_account_id],
            Vec::new(),
        ),
        EventPayload::AccountModeChanged(value) => (vec![value.account_id], Vec::new()),
        EventPayload::MarginModeChanged(value) => {
            (vec![value.account_id], vec![value.market_id.clone()])
        }
        EventPayload::LeverageChanged(value) => {
            (vec![value.account_id], vec![value.market_id.clone()])
        }
        EventPayload::InternalTransfer(_) => (vec![ACCOUNT_A, ACCOUNT_B], Vec::new()),
        EventPayload::AccountClassTransfer(_) | EventPayload::RewardClaimed(_) => {
            (vec![ACCOUNT_A], Vec::new())
        }
        EventPayload::SpotGenesisApplied(_) => (vec![ACCOUNT_A], Vec::new()),
        EventPayload::AssetContextUpdated(_) => (Vec::new(), Vec::new()),
        _ => unreachable!("test helper supports account and prerequisite payloads only"),
    }
}

fn raw_event(
    height: u64,
    event_index: u32,
    payload: EventPayload,
    market_ids: Vec<MarketId>,
    account_ids: Vec<Address>,
    schema: &str,
) -> CanonicalEventEnvelope {
    let payload_hash = *blake3::hash(&payload.encode_to_vec().unwrap()).as_bytes();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: schema.to_owned(),
        chain_id: ChainId::new("mainnet").unwrap(),
        block_height: BlockHeight::new(height),
        block_time: ProtocolTime::from_unix_micros(height as i64).unwrap(),
        transaction_id: TransactionId::new(format!("tx-{height}")).unwrap(),
        transaction_index: 0,
        canonical_event_index: event_index,
        market_ids,
        account_ids,
        source_evidence: vec![
            SourceEvidence::try_new_indexed(
                SourceId::new("test-primary").unwrap(),
                "v1",
                height.to_string(),
                payload_hash,
                event_index,
            )
            .unwrap(),
        ],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        ingested_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        canonicalized_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        parser_version: "test-parser-v1".to_owned(),
        payload,
    })
    .unwrap()
}

fn block(height: u64, events: Vec<CanonicalEventEnvelope>) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).unwrap(),
        ConfirmationClass::CommittedPrimary,
        events,
        BTreeMap::from([(SourceId::new("test-primary").unwrap(), [height as u8; 32])]),
    )
    .unwrap()
}

fn rebuild_with_identities(
    event: CanonicalEventEnvelope,
    accounts: Vec<Address>,
    markets: Vec<MarketId>,
) -> CanonicalEventEnvelope {
    raw_event(
        event.block_height().get(),
        event.canonical_event_index(),
        event.payload().clone(),
        markets,
        accounts,
        event.schema_version(),
    )
}

fn assert_reducer_reason<T: std::fmt::Debug>(
    result: Result<T, canonical_ledger::LedgerError>,
    expected: &str,
) {
    let error = result.unwrap_err();
    assert_eq!(error.reason_code(), "ledger.reducer_failed");
    assert_eq!(error.reducer_reason_code(), Some(expected));
}
