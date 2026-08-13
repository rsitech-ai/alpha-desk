use domain_types::{AccountId, ProtocolTime};
use entity_graph::{CounterpartyTrade, summarize_counterparty};

#[test]
fn counterparty_markout_controls_for_market_direction() {
    let a = AccountId::new("a").unwrap();
    let b = AccountId::new("b").unwrap();
    let trades = vec![
        CounterpartyTrade {
            maker: a.clone(),
            taker: b.clone(),
            maker_markout_bps: 10,
            market_return_bps: 10,
            inventory_transferred: true,
        },
        CounterpartyTrade {
            maker: a.clone(),
            taker: b.clone(),
            maker_markout_bps: -4,
            market_return_bps: 0,
            inventory_transferred: false,
        },
    ];
    let summary = summarize_counterparty(&trades, &a, &b).unwrap();
    assert_eq!(summary.sample_size, 2);
    assert_eq!(summary.inventory_transfer_count, 1);
    assert_eq!(summary.a_versus_b_markout_bps.raw(), -8);
}

#[test]
fn missing_pair_fails_closed() {
    let a = AccountId::new("a").unwrap();
    let b = AccountId::new("b").unwrap();
    let c = AccountId::new("c").unwrap();
    let trades = vec![CounterpartyTrade {
        maker: a.clone(),
        taker: c,
        maker_markout_bps: 1,
        market_return_bps: 0,
        inventory_transferred: false,
    }];
    assert!(summarize_counterparty(&trades, &a, &b).is_err());
    let _ = ProtocolTime::from_unix_micros(1).unwrap();
}
