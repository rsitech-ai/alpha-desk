use std::collections::BTreeSet;

use domain_types::{BlockHeight, MarketId, OrderId, OrderSide, Price, Quantity};
use serde::{Deserialize, Serialize};

use crate::book::{BookDiff, BookHealth, L2Level, OrderBook, RestingOrder};

pub const BOOK_FIXTURE_SCHEMA: &str = "hl.orderbook.fixture.v1";
pub const SYNTHETIC_UNASSESSED: &str = "synthetic_unassessed";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BookFixtureError {
    #[error("book fixture decode failed: {0}")]
    Decode(String),
    #[error("book fixture is unqualified-only: {0}")]
    Qualification(String),
    #[error("book fixture expected state mismatch: {0}")]
    ExpectedMismatch(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookFixture {
    pub schema: String,
    pub id: String,
    pub source_qualification: String,
    pub stage_1_qualified: bool,
    pub stage_2_qualified: bool,
    pub market_id: String,
    pub start_block: u64,
    pub price_scale: u8,
    pub quantity_scale: u8,
    pub ops: Vec<BookFixtureOp>,
    pub expected: BookFixtureExpected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
pub enum BookFixtureOp {
    Snapshot {
        sequence: u64,
        as_of_block: u64,
        orders: Vec<BookFixtureOrder>,
    },
    Diff {
        sequence: u64,
        as_of_block: u64,
        diff: BookFixtureDiff,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookFixtureOrder {
    pub order_id: String,
    pub side: OrderSide,
    pub price: String,
    pub remaining: String,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum BookFixtureDiff {
    Add {
        order: BookFixtureOrder,
    },
    Cancel {
        order_id: String,
    },
    Fill {
        order_id: String,
        fill_quantity: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookFixtureExpected {
    pub health: BookFixtureHealth,
    pub sequence: u64,
    pub as_of_block: u64,
    pub active_order_ids: Vec<String>,
    pub l2_bids: Vec<BookFixtureLevel>,
    pub l2_asks: Vec<BookFixtureLevel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum BookFixtureHealth {
    Healthy,
    AwaitingSnapshot { reason: String },
    Red { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookFixtureLevel {
    pub price: String,
    pub quantity: String,
    pub order_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookReplayReport {
    pub id: String,
    pub health: BookHealth,
    pub sequence: u64,
    pub as_of_block: BlockHeight,
    pub active_order_count: usize,
}

pub fn parse_book_fixture(json: &str) -> Result<BookFixture, BookFixtureError> {
    serde_json::from_str(json).map_err(|error| BookFixtureError::Decode(error.to_string()))
}

pub fn replay_book_fixture(fixture: &BookFixture) -> Result<BookReplayReport, BookFixtureError> {
    if fixture.schema != BOOK_FIXTURE_SCHEMA {
        return Err(BookFixtureError::Decode(format!(
            "unsupported schema {}",
            fixture.schema
        )));
    }
    if fixture.source_qualification != SYNTHETIC_UNASSESSED
        || fixture.stage_1_qualified
        || fixture.stage_2_qualified
    {
        return Err(BookFixtureError::Qualification(
            "fixtures must remain synthetic_unassessed with Stage 1/2 false".to_owned(),
        ));
    }
    let market = MarketId::new(&fixture.market_id)
        .map_err(|_| BookFixtureError::Decode("invalid market_id".to_owned()))?;
    let mut book = OrderBook::awaiting_snapshot(market, BlockHeight::new(fixture.start_block));
    for op in &fixture.ops {
        match op {
            BookFixtureOp::Snapshot {
                sequence,
                as_of_block,
                orders,
            } => {
                let decoded = orders
                    .iter()
                    .map(|order| decode_order(order, fixture.price_scale, fixture.quantity_scale))
                    .collect::<Result<Vec<_>, _>>()?;
                book.apply_snapshot(*sequence, BlockHeight::new(*as_of_block), decoded);
            }
            BookFixtureOp::Diff {
                sequence,
                as_of_block,
                diff,
            } => {
                book.apply_diff(
                    *sequence,
                    BlockHeight::new(*as_of_block),
                    decode_diff(diff, fixture.price_scale, fixture.quantity_scale)?,
                );
            }
        }
    }
    assert_expected(&book, fixture)?;
    Ok(BookReplayReport {
        id: fixture.id.clone(),
        health: book.health().clone(),
        sequence: book.sequence(),
        as_of_block: book.as_of_block(),
        active_order_count: book.active_orders().count(),
    })
}

fn decode_order(
    order: &BookFixtureOrder,
    price_scale: u8,
    quantity_scale: u8,
) -> Result<RestingOrder, BookFixtureError> {
    Ok(RestingOrder {
        order_id: OrderId::new(&order.order_id)
            .map_err(|_| BookFixtureError::Decode("invalid order_id".to_owned()))?,
        side: order.side,
        price: Price::parse_at_scale(&order.price, price_scale)
            .map_err(|_| BookFixtureError::Decode("invalid price".to_owned()))?,
        remaining: Quantity::parse_at_scale(&order.remaining, quantity_scale)
            .map_err(|_| BookFixtureError::Decode("invalid remaining".to_owned()))?,
        sequence: order.sequence,
    })
}

fn decode_diff(
    diff: &BookFixtureDiff,
    price_scale: u8,
    quantity_scale: u8,
) -> Result<BookDiff, BookFixtureError> {
    match diff {
        BookFixtureDiff::Add { order } => Ok(BookDiff::Add {
            order: decode_order(order, price_scale, quantity_scale)?,
        }),
        BookFixtureDiff::Cancel { order_id } => Ok(BookDiff::Cancel {
            order_id: OrderId::new(order_id)
                .map_err(|_| BookFixtureError::Decode("invalid cancel order_id".to_owned()))?,
        }),
        BookFixtureDiff::Fill {
            order_id,
            fill_quantity,
        } => Ok(BookDiff::Fill {
            order_id: OrderId::new(order_id)
                .map_err(|_| BookFixtureError::Decode("invalid fill order_id".to_owned()))?,
            fill_quantity: Quantity::parse_at_scale(fill_quantity, quantity_scale)
                .map_err(|_| BookFixtureError::Decode("invalid fill_quantity".to_owned()))?,
        }),
    }
}

fn assert_expected(book: &OrderBook, fixture: &BookFixture) -> Result<(), BookFixtureError> {
    let expected_health = match &fixture.expected.health {
        BookFixtureHealth::Healthy => BookHealth::Healthy,
        BookFixtureHealth::AwaitingSnapshot { reason } => BookHealth::AwaitingSnapshot {
            reason: reason.clone(),
        },
        BookFixtureHealth::Red { reason } => BookHealth::Red {
            reason: reason.clone(),
        },
    };
    if book.health() != &expected_health {
        return Err(BookFixtureError::ExpectedMismatch(format!(
            "health {:?} != {:?}",
            book.health(),
            expected_health
        )));
    }
    if book.sequence() != fixture.expected.sequence {
        return Err(BookFixtureError::ExpectedMismatch(
            "sequence mismatch".to_owned(),
        ));
    }
    if book.as_of_block() != BlockHeight::new(fixture.expected.as_of_block) {
        return Err(BookFixtureError::ExpectedMismatch(
            "as_of_block mismatch".to_owned(),
        ));
    }
    let actual_ids: BTreeSet<String> = book
        .active_orders()
        .map(|order| order.order_id.as_str().to_owned())
        .collect();
    let expected_ids: BTreeSet<String> =
        fixture.expected.active_order_ids.iter().cloned().collect();
    if actual_ids != expected_ids {
        return Err(BookFixtureError::ExpectedMismatch(
            "active order ids mismatch".to_owned(),
        ));
    }
    if !levels_match(
        &book.l2_bids(),
        &fixture.expected.l2_bids,
        fixture.price_scale,
        fixture.quantity_scale,
    ) || !levels_match(
        &book.l2_asks(),
        &fixture.expected.l2_asks,
        fixture.price_scale,
        fixture.quantity_scale,
    ) {
        return Err(BookFixtureError::ExpectedMismatch(
            "l2 levels mismatch".to_owned(),
        ));
    }
    Ok(())
}

fn levels_match(
    actual: &[L2Level],
    expected: &[BookFixtureLevel],
    price_scale: u8,
    quantity_scale: u8,
) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    actual.iter().zip(expected).all(|(level, expected)| {
        Price::parse_at_scale(&expected.price, price_scale)
            .ok()
            .is_some_and(|price| price == level.price)
            && Quantity::parse_at_scale(&expected.quantity, quantity_scale)
                .ok()
                .is_some_and(|quantity| quantity == level.quantity)
            && level.order_count == expected.order_count
    })
}
