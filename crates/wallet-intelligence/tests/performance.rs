use domain_types::{
    AccountId, AssetId, BasisPoints, BlockHeight, DexId, FeatureSetVersion, KnownTime, MarketId,
    ProtocolTime, RegimeId, UsdAmount,
};
use wallet_intelligence::{
    CashFlowKind, ConcentrationInput, DEFAULT_RETURN_SCALE, DEFAULT_USD_SCALE, EquityObservation,
    ExternalCashFlow, IntelligenceError, IntelligenceSubject, MarketBetaObservation,
    PerformanceLedger, concentration_breakdown, long_short_beta, long_short_beta_by_market,
    maker_taker_mix,
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
        collateral_pnl: vec![],
        regime_pnl: vec![],
        market_pnl: vec![],
        trade_pnl: vec![usd("90"), usd("10")],
        month_pnl: vec![usd("70"), usd("30")],
    })
    .unwrap();
    assert_eq!(breakdown.best_trade_share.ppm(), 900_000);
    assert_eq!(breakdown.best_month_share.ppm(), 700_000);
    assert!(breakdown.best_market_share.is_none());
    assert!(breakdown.asset_hhi_ppm.ppm() > 600_000);
    assert!(breakdown.collateral_hhi_ppm.is_none());
    assert!(breakdown.regime_hhi_ppm.is_none());
    assert_eq!(
        maker_taker_mix(usd("25"), usd("75")).unwrap().ppm(),
        250_000
    );
}

#[test]
fn collateral_and_regime_concentration_use_observed_series_only() {
    let breakdown = concentration_breakdown(&ConcentrationInput {
        asset_pnl: vec![(AssetId::new("btc").unwrap(), usd("100"))],
        dex_pnl: vec![(DexId::new("xyz").unwrap(), usd("100"))],
        collateral_pnl: vec![
            (AssetId::new("usdc").unwrap(), usd("80")),
            (AssetId::new("eth").unwrap(), usd("20")),
        ],
        regime_pnl: vec![
            (RegimeId::new("vol-high").unwrap(), usd("70")),
            (RegimeId::new("vol-low").unwrap(), usd("30")),
        ],
        market_pnl: vec![],
        trade_pnl: vec![usd("100")],
        month_pnl: vec![usd("100")],
    })
    .unwrap();
    assert_eq!(breakdown.collateral_hhi_ppm.unwrap().ppm(), 680_000);
    assert_eq!(breakdown.regime_hhi_ppm.unwrap().ppm(), 580_000);
    assert!(breakdown.best_market_share.is_none());
}

#[test]
fn best_market_share_uses_observed_market_series_and_withholds_when_unobserved() {
    let withheld = concentration_breakdown(&ConcentrationInput {
        asset_pnl: vec![(AssetId::new("btc").unwrap(), usd("100"))],
        dex_pnl: vec![(DexId::new("xyz").unwrap(), usd("100"))],
        collateral_pnl: vec![],
        regime_pnl: vec![],
        market_pnl: vec![],
        trade_pnl: vec![usd("100")],
        month_pnl: vec![usd("100")],
    })
    .unwrap();
    assert!(withheld.best_market_share.is_none());

    let breakdown = concentration_breakdown(&ConcentrationInput {
        asset_pnl: vec![(AssetId::new("btc").unwrap(), usd("100"))],
        dex_pnl: vec![(DexId::new("xyz").unwrap(), usd("100"))],
        collateral_pnl: vec![],
        regime_pnl: vec![],
        market_pnl: vec![
            (MarketId::new("BTC").unwrap(), usd("90")),
            (MarketId::new("ETH").unwrap(), usd("10")),
        ],
        trade_pnl: vec![usd("100")],
        month_pnl: vec![usd("100")],
    })
    .unwrap();
    assert_eq!(breakdown.best_market_share.unwrap().ppm(), 900_000);

    let duplicate = concentration_breakdown(&ConcentrationInput {
        asset_pnl: vec![(AssetId::new("btc").unwrap(), usd("100"))],
        dex_pnl: vec![(DexId::new("xyz").unwrap(), usd("100"))],
        collateral_pnl: vec![],
        regime_pnl: vec![],
        market_pnl: vec![
            (MarketId::new("BTC").unwrap(), usd("90")),
            (MarketId::new("BTC").unwrap(), usd("10")),
        ],
        trade_pnl: vec![usd("100")],
        month_pnl: vec![usd("100")],
    })
    .unwrap_err();
    assert!(matches!(
        duplicate,
        IntelligenceError::Malformed {
            what: "concentration",
            reason: "duplicate market"
        }
    ));

    let zero_total = concentration_breakdown(&ConcentrationInput {
        asset_pnl: vec![(AssetId::new("btc").unwrap(), usd("100"))],
        dex_pnl: vec![(DexId::new("xyz").unwrap(), usd("100"))],
        collateral_pnl: vec![],
        regime_pnl: vec![],
        market_pnl: vec![(MarketId::new("BTC").unwrap(), usd("0"))],
        trade_pnl: vec![usd("100")],
        month_pnl: vec![usd("100")],
    })
    .unwrap_err();
    assert!(matches!(zero_total, IntelligenceError::DivisionByZero));
}

#[test]
fn long_short_beta_by_market_withholds_missing_returns() {
    assert!(long_short_beta_by_market(&[]).unwrap().is_none());
    let withheld = long_short_beta_by_market(&[MarketBetaObservation {
        market_id: MarketId::new("BTC").unwrap(),
        long_pnl: usd("10"),
        short_pnl: usd("-4"),
        market_return: None,
    }])
    .unwrap();
    assert!(withheld.is_none());
    let zero = long_short_beta_by_market(&[MarketBetaObservation {
        market_id: MarketId::new("BTC").unwrap(),
        long_pnl: usd("10"),
        short_pnl: usd("-4"),
        market_return: Some(BasisPoints::from_raw(0, 2).unwrap()),
    }])
    .unwrap_err();
    assert!(matches!(
        zero,
        IntelligenceError::Unsupported {
            what: "beta_with_zero_market_return"
        }
    ));
    let duplicate = long_short_beta_by_market(&[
        MarketBetaObservation {
            market_id: MarketId::new("BTC").unwrap(),
            long_pnl: usd("10"),
            short_pnl: usd("-4"),
            market_return: Some(BasisPoints::from_raw(200, 2).unwrap()),
        },
        MarketBetaObservation {
            market_id: MarketId::new("BTC").unwrap(),
            long_pnl: usd("1"),
            short_pnl: usd("1"),
            market_return: Some(BasisPoints::from_raw(100, 2).unwrap()),
        },
    ])
    .unwrap_err();
    assert!(matches!(
        duplicate,
        IntelligenceError::Malformed {
            what: "beta",
            reason: "duplicate market"
        }
    ));
    let (long, short) =
        long_short_beta(usd("10"), usd("-4"), BasisPoints::from_raw(200, 2).unwrap()).unwrap();
    let by_market = long_short_beta_by_market(&[
        MarketBetaObservation {
            market_id: MarketId::new("ETH").unwrap(),
            long_pnl: usd("5"),
            short_pnl: usd("1"),
            market_return: None,
        },
        MarketBetaObservation {
            market_id: MarketId::new("BTC").unwrap(),
            long_pnl: usd("10"),
            short_pnl: usd("-4"),
            market_return: Some(BasisPoints::from_raw(200, 2).unwrap()),
        },
    ])
    .unwrap()
    .unwrap();
    assert_eq!(by_market.len(), 1);
    assert_eq!(by_market[0].market_id, MarketId::new("BTC").unwrap());
    assert_eq!(by_market[0].long_beta, long);
    assert_eq!(by_market[0].short_beta, short);
}
