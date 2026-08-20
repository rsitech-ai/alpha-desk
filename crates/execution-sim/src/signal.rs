use domain_types::{Direction, KnownTime, MarketId, Quantity, SignalId};
use serde::{Deserialize, Serialize};

use crate::clock::protocol_as_known;
use crate::error::SimError;
use crate::math::align_qty;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalSnapshot {
    signal_id: SignalId,
    market_id: MarketId,
    direction: Direction,
    detected_at: domain_types::ProtocolTime,
    known_at: KnownTime,
    requested_quantity: Quantity,
}

impl SignalSnapshot {
    pub fn new(
        signal_id: SignalId,
        market_id: MarketId,
        direction: Direction,
        detected_at: domain_types::ProtocolTime,
        known_at: KnownTime,
        requested_quantity: Quantity,
    ) -> Result<Self, SimError> {
        if direction == Direction::Flat {
            return Err(SimError::InvalidRequest {
                field: "signal.flat_direction",
            });
        }
        if requested_quantity.raw() <= 0 {
            return Err(SimError::InvalidRequest {
                field: "signal.quantity",
            });
        }
        if known_at < protocol_as_known(detected_at)? {
            return Err(SimError::InvalidRequest {
                field: "signal.known_before_detect",
            });
        }
        Ok(Self {
            signal_id,
            market_id,
            direction,
            detected_at,
            known_at,
            requested_quantity: align_qty(requested_quantity)?,
        })
    }

    pub fn refuse_future(&self, evaluation_known_at: KnownTime) -> Result<(), SimError> {
        if self.known_at > evaluation_known_at {
            return Err(SimError::FutureData {
                field: "signal.known_at",
            });
        }
        Ok(())
    }

    #[must_use]
    pub fn signal_id(&self) -> &SignalId {
        &self.signal_id
    }

    #[must_use]
    pub fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn direction(&self) -> Direction {
        self.direction
    }

    #[must_use]
    pub const fn detected_at(&self) -> domain_types::ProtocolTime {
        self.detected_at
    }

    #[must_use]
    pub const fn known_at(&self) -> KnownTime {
        self.known_at
    }

    #[must_use]
    pub const fn requested_quantity(&self) -> Quantity {
        self.requested_quantity
    }
}
