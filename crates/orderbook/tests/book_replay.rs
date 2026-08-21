use domain_types::{BlockHeight, MarketId, OrderId, OrderSide, Price, Quantity};
use orderbook::{BookDiff, BookHealth, OrderBook, RestingOrder};

#[test]
fn snapshot_and_contiguous_diffs_rebuild_l4_and_l2() {
    let market = MarketId::new("perp:BTC").unwrap();
    let mut book = OrderBook::awaiting_snapshot(market.clone(), BlockHeight::new(1));
    assert!(matches!(book.health(), BookHealth::AwaitingSnapshot { .. }));

    book.apply_snapshot(
        10,
        BlockHeight::new(10),
        vec![
            rest("bid-a", OrderSide::Buy, "100", "2", 1),
            rest("bid-b", OrderSide::Buy, "100", "1", 2),
            rest("ask-a", OrderSide::Sell, "101", "3", 3),
        ],
    );
    assert_eq!(*book.health(), BookHealth::Healthy);
    assert_eq!(book.l2_bids()[0].quantity, qty("3"));
    assert_eq!(book.l2_bids()[0].order_count, 2);
    assert_eq!(book.l2_asks()[0].quantity, qty("3"));
    assert_eq!(book.active_orders().count(), 3);

    book.apply_diff(
        11,
        BlockHeight::new(11),
        BookDiff::Fill {
            order_id: OrderId::new("ask-a").unwrap(),
            fill_quantity: qty("1"),
        },
    );
    assert_eq!(book.l2_asks()[0].quantity, qty("2"));
    book.apply_diff(
        12,
        BlockHeight::new(12),
        BookDiff::Cancel {
            order_id: OrderId::new("bid-b").unwrap(),
        },
    );
    assert_eq!(book.l2_bids()[0].quantity, qty("2"));
    assert_eq!(book.lifecycle().len(), 2);
}

#[test]
fn gaps_duplicates_negative_fills_and_crossed_books_are_red() {
    let market = MarketId::new("perp:ETH").unwrap();
    let mut book = OrderBook::awaiting_snapshot(market.clone(), BlockHeight::new(1));
    book.apply_snapshot(
        5,
        BlockHeight::new(5),
        vec![rest("bid", OrderSide::Buy, "100", "1", 1)],
    );

    book.apply_diff(
        7,
        BlockHeight::new(7),
        BookDiff::Add {
            order: rest("ask", OrderSide::Sell, "101", "1", 2),
        },
    );
    assert!(matches!(book.health(), BookHealth::Red { reason } if reason == "sequence gap"));

    book.apply_snapshot(
        8,
        BlockHeight::new(8),
        vec![
            rest("dup", OrderSide::Buy, "100", "1", 1),
            rest("dup", OrderSide::Buy, "99", "1", 2),
        ],
    );
    assert!(matches!(book.health(), BookHealth::Red { reason } if reason == "duplicate order id"));

    book.apply_snapshot(
        9,
        BlockHeight::new(9),
        vec![rest("live", OrderSide::Buy, "100", "1", 1)],
    );
    book.apply_diff(
        10,
        BlockHeight::new(10),
        BookDiff::Fill {
            order_id: OrderId::new("live").unwrap(),
            fill_quantity: qty("2"),
        },
    );
    assert!(matches!(book.health(), BookHealth::Red { .. }));

    book.apply_snapshot(
        11,
        BlockHeight::new(11),
        vec![
            rest("bid", OrderSide::Buy, "102", "1", 1),
            rest("ask", OrderSide::Sell, "101", "1", 2),
        ],
    );
    assert!(
        matches!(book.health(), BookHealth::Red { reason } if reason == "crossed or locked book")
    );
}

fn rest(id: &str, side: OrderSide, price: &str, remaining: &str, sequence: u64) -> RestingOrder {
    RestingOrder::new(
        OrderId::new(id).unwrap(),
        side,
        Price::parse_at_scale(price, 0).unwrap(),
        qty(remaining),
        sequence,
    )
}

fn qty(value: &str) -> Quantity {
    Quantity::parse_at_scale(value, 0).unwrap()
}
