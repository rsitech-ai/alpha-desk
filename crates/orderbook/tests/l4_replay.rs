use domain_types::{Address, BlockHeight, MarketId, OrderId, OrderSide, Price, Quantity};
use orderbook::{
    BookDiff, BookHealth, CF_CHECKPOINTS, CF_L2_BOOK, CF_L4_ORDERS, L2Level, L2ReconcileDecision,
    L2ReconcilePolicyV1, L4CheckpointV1, L4Error, L4Reconstruction, OrderBook, RestingOrder,
    TriggerKind, checkpoint_key, decode_l2_book, decode_resting_order, encode_l2_book,
    encode_resting_order, l2_book_key, l4_order_key, reconcile_derived_l2,
};

#[test]
fn snapshot_bootstrap_add_update_remove_rebuilds_l4_and_derived_l2() {
    let mut lane = L4Reconstruction::awaiting_snapshot(market(), BlockHeight::new(1));
    lane.apply_committed_snapshot(
        "snap-10",
        10,
        BlockHeight::new(10),
        vec![
            rest("bid-a", OrderSide::Buy, "100", "2", 1),
            rest("ask-a", OrderSide::Sell, "101", "3", 2),
        ],
    )
    .unwrap();
    assert_eq!(*lane.book().health(), BookHealth::Healthy);

    lane.apply_committed_diff(
        "new-11",
        11,
        BlockHeight::new(11),
        BookDiff::Add {
            order: rest("bid-b", OrderSide::Buy, "100", "1", 3).with_time_millis(3),
        },
    )
    .unwrap();
    lane.apply_committed_diff(
        "upd-12",
        12,
        BlockHeight::new(12),
        BookDiff::Update {
            order_id: OrderId::new("ask-a").unwrap(),
            remaining: qty("1"),
            price: price("101"),
        },
    )
    .unwrap();
    lane.apply_committed_diff(
        "rm-13",
        13,
        BlockHeight::new(13),
        BookDiff::Cancel {
            order_id: OrderId::new("bid-b").unwrap(),
        },
    )
    .unwrap();

    assert_eq!(lane.book().l2_bids()[0].quantity, qty("2"));
    assert_eq!(lane.book().l2_asks()[0].quantity, qty("1"));
    assert_eq!(lane.book().active_order_count(), 2);
}

#[test]
fn committed_fifo_at_price_is_time_priority_and_trigger_orders_stay_off_l2() {
    let mut book = OrderBook::awaiting_snapshot(market(), BlockHeight::new(1));
    book.apply_snapshot(
        1,
        BlockHeight::new(10),
        vec![
            rest("late", OrderSide::Buy, "100", "1", 2).with_time_millis(20),
            rest("early", OrderSide::Buy, "100", "1", 1).with_time_millis(10),
            rest("trig", OrderSide::Buy, "100", "9", 3)
                .with_time_millis(5)
                .with_trigger(TriggerKind::Untriggered {
                    tpsl: true,
                    trigger_px: price("90"),
                }),
            rest("ask", OrderSide::Sell, "101", "1", 4),
        ],
    );
    assert_eq!(book.best_bid().unwrap().order_id.as_str(), "early");
    assert_eq!(book.l2_bids()[0].quantity, qty("2"));
    assert_eq!(book.l2_bids()[0].order_count, 2);
    assert!(
        book.active_orders()
            .any(|order| order.order_id.as_str() == "trig" && order.trigger.is_untriggered())
    );
}

#[test]
fn duplicate_committed_inputs_are_idempotent_and_reordered_provisional_never_mutates_l4() {
    let mut lane = L4Reconstruction::awaiting_snapshot(market(), BlockHeight::new(1));
    let snapshot = vec![rest("bid", OrderSide::Buy, "100", "1", 1)];
    lane.apply_committed_snapshot("snap", 5, BlockHeight::new(5), snapshot.clone())
        .unwrap();
    let before = lane.book().sequence();
    lane.apply_committed_snapshot("snap", 5, BlockHeight::new(5), snapshot)
        .unwrap();
    assert_eq!(lane.book().sequence(), before);

    let conflict = lane.apply_committed_snapshot(
        "snap",
        5,
        BlockHeight::new(5),
        vec![rest("other", OrderSide::Buy, "99", "1", 1)],
    );
    assert!(matches!(conflict, Err(L4Error::Conflict { .. })));

    let hash_a = [1_u8; 32];
    let hash_b = [2_u8; 32];
    lane.observe_provisional("ws-15", hash_a).unwrap();
    lane.observe_provisional("ws-13", hash_b).unwrap();
    lane.observe_provisional("ws-15", hash_a).unwrap();
    assert_eq!(lane.book().sequence(), before);
    assert_eq!(lane.provisional_count(), 2);
    assert_eq!(*lane.book().health(), BookHealth::Healthy);
}

#[test]
fn derived_l2_matches_official_under_v1_policy_and_quarantines_divergence() {
    let derived = vec![L2Level {
        price: price("100"),
        quantity: qty("3"),
        order_count: 2,
    }];
    let official = derived.clone();
    assert_eq!(
        reconcile_derived_l2(
            &derived,
            &[],
            &official,
            &[],
            Some(1_000),
            Some(1_500),
            &L2ReconcilePolicyV1::for_market(price("1"), qty("1")),
        ),
        L2ReconcileDecision::Match
    );

    let skewed = reconcile_derived_l2(
        &derived,
        &[],
        &official,
        &[],
        Some(1_000),
        Some(4_000),
        &L2ReconcilePolicyV1::for_market(price("1"), qty("1")),
    );
    assert!(matches!(
        skewed,
        L2ReconcileDecision::Quarantine { reason } if reason.contains("timing")
    ));

    let mut wrong_n = official.clone();
    wrong_n[0].order_count = 1;
    assert!(matches!(
        reconcile_derived_l2(
            &derived,
            &[],
            &wrong_n,
            &[],
            None,
            None,
            &L2ReconcilePolicyV1::exact(),
        ),
        L2ReconcileDecision::Quarantine { .. }
    ));
}

#[test]
fn checkpoint_restore_yields_identical_hash_and_rocksdb_encoding_round_trips() {
    let mut book = OrderBook::awaiting_snapshot(market(), BlockHeight::new(1));
    book.apply_snapshot(
        8,
        BlockHeight::new(8),
        vec![
            rest("bid", OrderSide::Buy, "100", "2", 1).with_time_millis(11),
            rest("ask", OrderSide::Sell, "101", "3", 2),
        ],
    );
    let captured = L4CheckpointV1::capture(&book).unwrap();
    let restored = captured.restore().unwrap();
    assert_eq!(
        L4CheckpointV1::capture(&restored).unwrap().state_hash(),
        captured.state_hash()
    );
    assert_eq!(restored.l2_bids(), book.l2_bids());
    assert_eq!(restored.l2_asks(), book.l2_asks());

    let encoded = captured.encode().unwrap();
    let decoded = L4CheckpointV1::decode(&encoded).unwrap();
    assert_eq!(decoded.state_hash(), captured.state_hash());

    let order = rest("bid", OrderSide::Buy, "100", "2", 1);
    let order_bytes = encode_resting_order(&order).unwrap();
    assert_eq!(decode_resting_order(&order_bytes).unwrap(), order);
    let l2_bytes = encode_l2_book(&book.l2_bids(), &book.l2_asks()).unwrap();
    let (bids, asks) = decode_l2_book(&l2_bytes).unwrap();
    assert_eq!(bids, book.l2_bids());
    assert_eq!(asks, book.l2_asks());

    assert_ne!(
        l4_order_key(&market(), &OrderId::new("bid").unwrap()),
        l2_book_key(&market())
    );
    assert_ne!(
        checkpoint_key(&market(), BlockHeight::new(8)),
        l2_book_key(&market())
    );
    assert_eq!(CF_L4_ORDERS, "l4_orders");
    assert_eq!(CF_L2_BOOK, "l2_book");
    assert_eq!(CF_CHECKPOINTS, "checkpoints");
}

#[test]
fn memory_stays_bounded_under_synthetic_high_order_count() {
    let cap = 4_096;
    let mut book = OrderBook::awaiting_snapshot_bounded(market(), BlockHeight::new(1), cap);
    let mut orders = Vec::with_capacity(cap);
    for index in 0..cap {
        orders.push(rest(
            &format!("o-{index}"),
            OrderSide::Buy,
            "100",
            "1",
            index as u64,
        ));
    }
    book.apply_snapshot(1, BlockHeight::new(1), orders);
    assert_eq!(*book.health(), BookHealth::Healthy);
    assert_eq!(book.active_order_count(), cap);

    book.apply_diff(
        2,
        BlockHeight::new(2),
        BookDiff::Add {
            order: rest("overflow", OrderSide::Sell, "101", "1", cap as u64),
        },
    );
    assert!(matches!(
        book.health(),
        BookHealth::Red { reason } if reason == "order count bound"
    ));
    assert_eq!(book.active_order_count(), 0);

    let mut lane = L4Reconstruction::awaiting_snapshot_bounded(market(), BlockHeight::new(1), 8, 2);
    lane.apply_committed_snapshot(
        "s",
        1,
        BlockHeight::new(1),
        vec![rest("a", OrderSide::Buy, "1", "1", 1)],
    )
    .unwrap();
    lane.observe_provisional("p1", [1; 32]).unwrap();
    lane.observe_provisional("p2", [2; 32]).unwrap();
    assert!(matches!(
        lane.observe_provisional("p3", [3; 32]),
        Err(L4Error::ProvisionalRefused(_))
    ));
    assert_eq!(*lane.book().health(), BookHealth::Healthy);
}

#[test]
fn same_id_payload_with_different_side_or_account_conflicts() {
    let account_a = Address::parse_api("0x00000000000000000000000000000000000000aa").unwrap();
    let account_b = Address::parse_api("0x00000000000000000000000000000000000000bb").unwrap();

    let mut snapshot_side = L4Reconstruction::awaiting_snapshot(market(), BlockHeight::new(1));
    snapshot_side
        .apply_committed_snapshot(
            "snap",
            5,
            BlockHeight::new(5),
            vec![rest("oid", OrderSide::Buy, "100", "1", 1).with_account(account_a)],
        )
        .unwrap();
    assert!(matches!(
        snapshot_side.apply_committed_snapshot(
            "snap",
            5,
            BlockHeight::new(5),
            vec![rest("oid", OrderSide::Sell, "100", "1", 1).with_account(account_a)],
        ),
        Err(L4Error::Conflict { .. })
    ));

    let mut snapshot_account = L4Reconstruction::awaiting_snapshot(market(), BlockHeight::new(1));
    snapshot_account
        .apply_committed_snapshot(
            "snap",
            5,
            BlockHeight::new(5),
            vec![rest("oid", OrderSide::Buy, "100", "1", 1).with_account(account_a)],
        )
        .unwrap();
    assert!(matches!(
        snapshot_account.apply_committed_snapshot(
            "snap",
            5,
            BlockHeight::new(5),
            vec![rest("oid", OrderSide::Buy, "100", "1", 1).with_account(account_b)],
        ),
        Err(L4Error::Conflict { .. })
    ));

    let mut add = L4Reconstruction::awaiting_snapshot(market(), BlockHeight::new(1));
    add.apply_committed_snapshot("boot", 1, BlockHeight::new(1), Vec::new())
        .unwrap();
    add.apply_committed_diff(
        "add",
        2,
        BlockHeight::new(2),
        BookDiff::Add {
            order: rest("oid", OrderSide::Buy, "100", "1", 2).with_account(account_a),
        },
    )
    .unwrap();
    assert!(matches!(
        add.apply_committed_diff(
            "add",
            2,
            BlockHeight::new(2),
            BookDiff::Add {
                order: rest("oid", OrderSide::Sell, "100", "1", 2).with_account(account_a),
            },
        ),
        Err(L4Error::Conflict { .. })
    ));
}

fn market() -> MarketId {
    MarketId::new("perp:BTC").unwrap()
}

fn rest(id: &str, side: OrderSide, px: &str, remaining: &str, sequence: u64) -> RestingOrder {
    RestingOrder::new(
        OrderId::new(id).unwrap(),
        side,
        price(px),
        qty(remaining),
        sequence,
    )
}

fn price(value: &str) -> Price {
    Price::parse_at_scale(value, 0).unwrap()
}

fn qty(value: &str) -> Quantity {
    Quantity::parse_at_scale(value, 0).unwrap()
}
