use std::cmp::Reverse;
use std::collections::{BTreeMap, btree_map::Entry};

use domain_types::{
    Address, BlockHeight, ClientOrderId, MarketId, OrderId, OrderSide, Price, Quantity,
};

pub const DEFAULT_MAX_ORDERS: usize = 1_048_576;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookHealth {
    Healthy,
    AwaitingSnapshot { reason: String },
    Red { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerKind {
    None,
    Untriggered { tpsl: bool, trigger_px: Price },
    Activated { trigger_px: Price },
}

impl TriggerKind {
    #[must_use]
    pub const fn is_untriggered(&self) -> bool {
        matches!(self, Self::Untriggered { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestingOrder {
    pub order_id: OrderId,
    pub side: OrderSide,
    pub price: Price,
    pub remaining: Quantity,
    pub original: Quantity,
    pub sequence: u64,
    pub account_id: Option<Address>,
    pub client_order_id: Option<ClientOrderId>,
    pub time_millis: Option<u64>,
    pub trigger: TriggerKind,
}

impl RestingOrder {
    #[must_use]
    pub fn new(
        order_id: OrderId,
        side: OrderSide,
        price: Price,
        remaining: Quantity,
        sequence: u64,
    ) -> Self {
        Self {
            order_id,
            side,
            price,
            remaining,
            original: remaining,
            sequence,
            account_id: None,
            client_order_id: None,
            time_millis: None,
            trigger: TriggerKind::None,
        }
    }

    #[must_use]
    pub fn with_original(mut self, original: Quantity) -> Self {
        self.original = original;
        self
    }

    #[must_use]
    pub const fn with_account(mut self, account_id: Address) -> Self {
        self.account_id = Some(account_id);
        self
    }

    #[must_use]
    pub fn with_client_order_id(mut self, client_order_id: ClientOrderId) -> Self {
        self.client_order_id = Some(client_order_id);
        self
    }

    #[must_use]
    pub const fn with_time_millis(mut self, time_millis: u64) -> Self {
        self.time_millis = Some(time_millis);
        self
    }

    #[must_use]
    pub fn with_trigger(mut self, trigger: TriggerKind) -> Self {
        self.trigger = trigger;
        self
    }

    #[must_use]
    pub fn time_priority(&self) -> u64 {
        self.time_millis.unwrap_or(self.sequence)
    }

    #[must_use]
    pub const fn rests_on_l2(&self) -> bool {
        !self.trigger.is_untriggered()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookDiff {
    Add {
        order: RestingOrder,
    },
    Update {
        order_id: OrderId,
        remaining: Quantity,
        price: Price,
    },
    Cancel {
        order_id: OrderId,
    },
    Fill {
        order_id: OrderId,
        fill_quantity: Quantity,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L2Level {
    pub price: Price,
    pub quantity: Quantity,
    pub order_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleEvent {
    pub order_id: OrderId,
    pub kind: LifecycleKind,
    pub sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleKind {
    Added,
    Updated,
    Filled,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderBook {
    market_id: MarketId,
    sequence: u64,
    as_of_block: BlockHeight,
    health: BookHealth,
    max_orders: usize,
    bids: BTreeMap<(Reverse<Price>, u64, OrderId), RestingOrder>,
    asks: BTreeMap<(Price, u64, OrderId), RestingOrder>,
    by_id: BTreeMap<OrderId, OrderIndex>,
    lifecycle: Vec<LifecycleEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OrderIndex {
    side: OrderSide,
    price: Price,
    time_priority: u64,
}

impl OrderBook {
    pub fn awaiting_snapshot(market_id: MarketId, as_of_block: BlockHeight) -> Self {
        Self::awaiting_snapshot_bounded(market_id, as_of_block, DEFAULT_MAX_ORDERS)
    }

    pub fn awaiting_snapshot_bounded(
        market_id: MarketId,
        as_of_block: BlockHeight,
        max_orders: usize,
    ) -> Self {
        Self {
            market_id,
            sequence: 0,
            as_of_block,
            health: BookHealth::AwaitingSnapshot {
                reason: "empty book requires a verified snapshot".to_owned(),
            },
            max_orders,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            by_id: BTreeMap::new(),
            lifecycle: Vec::new(),
        }
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn as_of_block(&self) -> BlockHeight {
        self.as_of_block
    }

    #[must_use]
    pub const fn health(&self) -> &BookHealth {
        &self.health
    }

    #[must_use]
    pub const fn max_orders(&self) -> usize {
        self.max_orders
    }

    #[must_use]
    pub fn active_order_count(&self) -> usize {
        self.by_id.len()
    }

    pub fn active_orders(&self) -> impl Iterator<Item = &RestingOrder> {
        self.bids.values().chain(self.asks.values())
    }

    #[must_use]
    pub fn lifecycle(&self) -> &[LifecycleEvent] {
        &self.lifecycle
    }

    pub fn apply_snapshot(
        &mut self,
        sequence: u64,
        as_of_block: BlockHeight,
        orders: Vec<RestingOrder>,
    ) {
        self.bids.clear();
        self.asks.clear();
        self.by_id.clear();
        self.lifecycle.clear();
        self.sequence = sequence;
        self.as_of_block = as_of_block;
        self.health = BookHealth::Healthy;
        if orders.len() > self.max_orders {
            self.mark_red("order count bound");
            return;
        }
        for order in orders {
            if let Err(reason) = self.insert_active(order, true) {
                self.mark_red(reason);
                return;
            }
        }
        if let Err(reason) = self.assert_invariants() {
            self.mark_red(reason);
        }
    }

    pub fn apply_diff(&mut self, sequence: u64, as_of_block: BlockHeight, diff: BookDiff) {
        if !matches!(self.health, BookHealth::Healthy) {
            self.mark_red("diff applied while book is not healthy");
            return;
        }
        if sequence != self.sequence.saturating_add(1) {
            self.mark_red("sequence gap");
            return;
        }
        self.sequence = sequence;
        self.as_of_block = as_of_block;
        if let Err(reason) = self.apply_healthy_diff(diff) {
            self.mark_red(reason);
            return;
        }
        if let Err(reason) = self.assert_invariants() {
            self.mark_red(reason);
        }
    }

    #[must_use]
    pub fn l2_bids(&self) -> Vec<L2Level> {
        aggregate_l2(self.bids.values(), true).unwrap_or_default()
    }

    #[must_use]
    pub fn l2_asks(&self) -> Vec<L2Level> {
        aggregate_l2(self.asks.values(), false).unwrap_or_default()
    }

    #[must_use]
    pub fn best_bid(&self) -> Option<&RestingOrder> {
        self.bids.values().find(|order| order.rests_on_l2())
    }

    #[must_use]
    pub fn best_ask(&self) -> Option<&RestingOrder> {
        self.asks.values().find(|order| order.rests_on_l2())
    }

    fn apply_healthy_diff(&mut self, diff: BookDiff) -> Result<(), String> {
        match diff {
            BookDiff::Add { order } => self.insert_active(order, false),
            BookDiff::Update {
                order_id,
                remaining,
                price,
            } => self.update_active(&order_id, remaining, price),
            BookDiff::Cancel { order_id } => {
                self.remove_active(&order_id, LifecycleKind::Cancelled)
            }
            BookDiff::Fill {
                order_id,
                fill_quantity,
            } => self.fill_active(&order_id, fill_quantity),
        }
    }

    fn insert_active(&mut self, order: RestingOrder, from_snapshot: bool) -> Result<(), String> {
        if order.remaining.raw() <= 0 || order.price.raw() <= 0 || order.original.raw() <= 0 {
            return Err("negative or non-positive order quantity or price".to_owned());
        }
        if self.by_id.contains_key(&order.order_id) {
            return Err("duplicate order id".to_owned());
        }
        if self.by_id.len() >= self.max_orders {
            return Err("order count bound".to_owned());
        }
        let index = OrderIndex {
            side: order.side,
            price: order.price,
            time_priority: order.time_priority(),
        };
        match order.side {
            OrderSide::Buy => {
                self.bids.insert(bid_key(&order), order.clone());
            }
            OrderSide::Sell => {
                self.asks.insert(ask_key(&order), order.clone());
            }
        }
        self.by_id.insert(order.order_id.clone(), index);
        if !from_snapshot {
            self.lifecycle.push(LifecycleEvent {
                order_id: order.order_id,
                kind: LifecycleKind::Added,
                sequence: self.sequence,
            });
        }
        Ok(())
    }

    fn update_active(
        &mut self,
        order_id: &OrderId,
        remaining: Quantity,
        price: Price,
    ) -> Result<(), String> {
        if remaining.raw() < 0 || price.raw() <= 0 {
            return Err("negative or non-positive order quantity or price".to_owned());
        }
        let Some(index) = self.by_id.get(order_id).cloned() else {
            return Err("order id is not active".to_owned());
        };
        let mut order = self
            .take_active(order_id, &index)
            .ok_or_else(|| "order index is inconsistent".to_owned())?;
        order.remaining = remaining;
        order.price = price;
        if remaining.raw() == 0 {
            self.lifecycle.push(LifecycleEvent {
                order_id: order_id.clone(),
                kind: LifecycleKind::Filled,
                sequence: self.sequence,
            });
            return Ok(());
        }
        let new_index = OrderIndex {
            side: order.side,
            price: order.price,
            time_priority: order.time_priority(),
        };
        match order.side {
            OrderSide::Buy => {
                self.bids.insert(bid_key(&order), order);
            }
            OrderSide::Sell => {
                self.asks.insert(ask_key(&order), order);
            }
        }
        self.by_id.insert(order_id.clone(), new_index);
        self.lifecycle.push(LifecycleEvent {
            order_id: order_id.clone(),
            kind: LifecycleKind::Updated,
            sequence: self.sequence,
        });
        Ok(())
    }

    fn take_active(&mut self, order_id: &OrderId, index: &OrderIndex) -> Option<RestingOrder> {
        self.by_id.remove(order_id)?;
        match index.side {
            OrderSide::Buy => {
                self.bids
                    .remove(&(Reverse(index.price), index.time_priority, order_id.clone()))
            }
            OrderSide::Sell => {
                self.asks
                    .remove(&(index.price, index.time_priority, order_id.clone()))
            }
        }
    }

    fn remove_active(&mut self, order_id: &OrderId, kind: LifecycleKind) -> Result<(), String> {
        let Some(index) = self.by_id.get(order_id).cloned() else {
            return Err("order id is not active".to_owned());
        };
        self.take_active(order_id, &index)
            .ok_or_else(|| "order index is inconsistent".to_owned())?;
        self.lifecycle.push(LifecycleEvent {
            order_id: order_id.clone(),
            kind,
            sequence: self.sequence,
        });
        Ok(())
    }

    fn fill_active(&mut self, order_id: &OrderId, fill_quantity: Quantity) -> Result<(), String> {
        if fill_quantity.raw() <= 0 {
            return Err("fill quantity must be positive".to_owned());
        }
        let Some(index) = self.by_id.get(order_id).cloned() else {
            return Err("order id is not active".to_owned());
        };
        let remaining = match index.side {
            OrderSide::Buy => {
                let order = self
                    .bids
                    .get_mut(&(Reverse(index.price), index.time_priority, order_id.clone()))
                    .ok_or_else(|| "bid index is inconsistent".to_owned())?;
                apply_fill_qty(order, fill_quantity)?
            }
            OrderSide::Sell => {
                let order = self
                    .asks
                    .get_mut(&(index.price, index.time_priority, order_id.clone()))
                    .ok_or_else(|| "ask index is inconsistent".to_owned())?;
                apply_fill_qty(order, fill_quantity)?
            }
        };
        if remaining.raw() == 0 {
            self.remove_active(order_id, LifecycleKind::Filled)?;
        } else {
            self.lifecycle.push(LifecycleEvent {
                order_id: order_id.clone(),
                kind: LifecycleKind::Filled,
                sequence: self.sequence,
            });
        }
        Ok(())
    }

    fn assert_invariants(&self) -> Result<(), String> {
        if self.by_id.len() != self.bids.len() + self.asks.len() {
            return Err("active index is inconsistent".to_owned());
        }
        if let (Some(bid), Some(ask)) = (self.best_bid(), self.best_ask())
            && bid.price >= ask.price
        {
            return Err("crossed or locked book".to_owned());
        }
        aggregate_l2(self.bids.values(), true)?;
        aggregate_l2(self.asks.values(), false)?;
        Ok(())
    }

    fn mark_red(&mut self, reason: impl Into<String>) {
        self.bids.clear();
        self.asks.clear();
        self.by_id.clear();
        self.health = BookHealth::Red {
            reason: reason.into(),
        };
    }
}

fn apply_fill_qty(order: &mut RestingOrder, fill_quantity: Quantity) -> Result<Quantity, String> {
    let remaining = order
        .remaining
        .checked_sub(fill_quantity)
        .map_err(|_| "fill exceeds remaining quantity".to_owned())?;
    if remaining.raw() < 0 {
        return Err("negative remaining quantity".to_owned());
    }
    order.remaining = remaining;
    Ok(remaining)
}

fn bid_key(order: &RestingOrder) -> (Reverse<Price>, u64, OrderId) {
    (
        Reverse(order.price),
        order.time_priority(),
        order.order_id.clone(),
    )
}

fn ask_key(order: &RestingOrder) -> (Price, u64, OrderId) {
    (order.price, order.time_priority(), order.order_id.clone())
}

fn aggregate_l2<'a>(
    orders: impl Iterator<Item = &'a RestingOrder>,
    bids: bool,
) -> Result<Vec<L2Level>, String> {
    let mut levels: BTreeMap<Price, (Quantity, u32)> = BTreeMap::new();
    for order in orders {
        if !order.rests_on_l2() {
            continue;
        }
        match levels.entry(order.price) {
            Entry::Vacant(vacant) => {
                let zero = Quantity::from_raw(0, order.remaining.scale())
                    .map_err(|_| "invalid quantity scale".to_owned())?;
                vacant.insert((zero, 0));
            }
            Entry::Occupied(_) => {}
        }
        let entry = levels
            .get_mut(&order.price)
            .ok_or_else(|| "l2 level index is inconsistent".to_owned())?;
        entry.0 = entry
            .0
            .checked_add(order.remaining)
            .map_err(|_| "l2 aggregation overflow".to_owned())?;
        entry.1 = entry
            .1
            .checked_add(1)
            .ok_or_else(|| "l2 order count overflow".to_owned())?;
    }
    let mut out: Vec<L2Level> = levels
        .into_iter()
        .map(|(price, (quantity, order_count))| L2Level {
            price,
            quantity,
            order_count,
        })
        .collect();
    if bids {
        out.reverse();
    }
    Ok(out)
}
