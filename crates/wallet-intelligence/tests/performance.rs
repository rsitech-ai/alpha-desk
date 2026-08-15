use domain_types::{
    AccountId, AssetId, BlockHeight, DexId, FeatureSetVersion, KnownTime, ProtocolTime, UsdAmount,
};
use wallet_intelligence::{
    CashFlowKind, ConcentrationInput, DEFAULT_RETURN_SCALE, DEFAULT_USD_SCALE, EquityObservation,
    ExternalCashFlow, IntelligenceError, IntelligenceSubject, PerformanceLedger,
    concentration_breakdown, maker_taker_mix,
};

fn usd(value: &str) -> UsdAmount {
    UsdAmount::parse_at_scale(value, DEFAULT_USD_SCALE).unwrap()
}

fn time(seconds: i64) -> ProtocolTime {
    ProtocolTime::from_unix_micros(seconds * 1_000_000).unwrap()
}

fn observation(seconds: i64, equity: &str) -> EquityObservation {
    EquityObservation::try_new(
        time(seconds),
        usd(equity),
        usd("0"),
        usd("0"),
        usd("0"),
        usd("0"),
        usd("50"),
        usd("20"),
    )
    .unwrap()
}

#[test]
fn cash_flow_adjusted_return_excludes_deposits_from_trading_gain() {
    let mut ledger = PerformanceLedger::try_new(
        IntelligenceSubject::Account(AccountId::new("acct-a").unwrap()),
        DEFAULT_USD_SCALE,
        DEFAULT_RETURN_SCALE,
    )
    .unwrap();
    ledger.observe(observation(1, "100")).unwrap();
    ledger.observe(observation(2, "110")).unwrap();
    ledger
        .apply_cash_flow(
            ExternalCashFlow::try_new(time(3), CashFlowKind::Deposit, usd("100")).unwrap(),
        )
        .unwrap();
    ledger.observe(observation(4, "215")).unwrap();
    let snapshot = ledger
        .snapshot(
            FeatureSetVersion::new("wallet-v1").unwrap(),
            KnownTime::from_unix_micros(4_000_000).unwrap(),
            BlockHeight::new(4),
            None,
        )
        .unwrap();
    assert_eq!(snapshot.trading_gain, usd("15"));
    assert_ne!(snapshot.trading_gain, usd("115"));
    assert_eq!(snapshot.net_external_cash_flow, usd("100"));
    assert_eq!(snapshot.starting_equity, usd("100"));
    assert_eq!(snapshot.ending_equity, usd("215"));
    let expected_twr = domain_types::Decimal::from_raw(53, 0)
        .unwrap()
        .checked_div(
            domain_types::Decimal::from_raw(420, 0).unwrap(),
            DEFAULT_RETURN_SCALE,
            domain_types::RoundingMode::NearestTiesToEven,
        )
        .unwrap();
    assert_eq!(snapshot.time_weighted_return, expected_twr);
}

#[test]
fn as_of_snapshot_ignores_later_cash_flows() {
    let mut ledger = PerformanceLedger::try_new(
        IntelligenceSubject::Account(AccountId::new("acct-a").unwrap()),
        DEFAULT_USD_SCALE,
        DEFAULT_RETURN_SCALE,
    )
    .unwrap();
    ledger.observe(observation(1, "100")).unwrap();
    ledger.observe(observation(2, "110")).unwrap();
    ledger
        .apply_cash_flow(
            ExternalCashFlow::try_new(time(3), CashFlowKind::Deposit, usd("100")).unwrap(),
        )
        .unwrap();
    let early = ledger
        .snapshot(
            FeatureSetVersion::new("wallet-v1").unwrap(),
            KnownTime::from_unix_micros(2_000_000).unwrap(),
            BlockHeight::new(2),
            Some(time(2)),
        )
        .unwrap();
    assert_eq!(early.trading_gain, usd("10"));
    assert_eq!(early.net_external_cash_flow, usd("0"));
}

#[test]
fn malformed_and_unsupported_inputs_fail_closed() {
    let mut ledger = PerformanceLedger::try_new(
        IntelligenceSubject::Account(AccountId::new("acct-a").unwrap()),
        DEFAULT_USD_SCALE,
        DEFAULT_RETURN_SCALE,
    )
    .unwrap();
    ledger.observe(observation(2, "100")).unwrap();
    let error = ledger.observe(observation(1, "90")).unwrap_err();
    assert!(matches!(error, IntelligenceError::Malformed { .. }));
    let scale_error = PerformanceLedger::try_new(
        IntelligenceSubject::Account(AccountId::new("acct-a").unwrap()),
        0,
        DEFAULT_RETURN_SCALE,
    )
    .unwrap_err();
    assert!(matches!(scale_error, IntelligenceError::Malformed { .. }));
}

#[test]
fn concentration_and_maker_mix_preserve_exact_ratios() {
    let breakdown = concentration_breakdown(&ConcentrationInput {
        asset_pnl: vec![
            (AssetId::new("btc").unwrap(), usd("80")),
            (AssetId::new("eth").unwrap(), usd("20")),
        ],
        dex_pnl: vec![(DexId::new("xyz").unwrap(), usd("100"))],
        trade_pnl: vec![usd("90"), usd("10")],
        month_pnl: vec![usd("70"), usd("30")],
    })
    .unwrap();
    assert_eq!(breakdown.best_trade_share.ppm(), 900_000);
    assert_eq!(breakdown.best_month_share.ppm(), 700_000);
    assert!(breakdown.asset_hhi_ppm.ppm() > 600_000);
    assert_eq!(
        maker_taker_mix(usd("25"), usd("75")).unwrap().ppm(),
        250_000
    );
}
