use domain_types::{Direction, Price};
use serde::{Deserialize, Serialize};

use crate::error::SimError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderType {
    Market,
    Ioc,
    Gtc,
    Alo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderPolicy {
    order_type: OrderType,
    limit_price: Option<Price>,
    queue_fill_min_ppm: Option<u32>,
    queue_fill_max_ppm: Option<u32>,
}

impl OrderPolicy {
    pub fn market() -> Self {
        Self {
            order_type: OrderType::Market,
            limit_price: None,
            queue_fill_min_ppm: None,
            queue_fill_max_ppm: None,
        }
    }

    pub fn ioc() -> Self {
        Self {
            order_type: OrderType::Ioc,
            limit_price: None,
            queue_fill_min_ppm: None,
            queue_fill_max_ppm: None,
        }
    }

    pub fn ioc_limit(limit_price: Price) -> Self {
        Self {
            order_type: OrderType::Ioc,
            limit_price: Some(limit_price),
            queue_fill_min_ppm: None,
            queue_fill_max_ppm: None,
        }
    }

    pub fn gtc(
        limit_price: Price,
        queue_fill_min_ppm: u32,
        queue_fill_max_ppm: u32,
    ) -> Result<Self, SimError> {
        if queue_fill_min_ppm > queue_fill_max_ppm || queue_fill_max_ppm > 1_000_000 {
            return Err(SimError::UnmodeledCost {
                component: "gtc_queue_model",
            });
        }
        Ok(Self {
            order_type: OrderType::Gtc,
            limit_price: Some(limit_price),
            queue_fill_min_ppm: Some(queue_fill_min_ppm),
            queue_fill_max_ppm: Some(queue_fill_max_ppm),
        })
    }

    pub fn alo(limit_price: Price) -> Self {
        Self {
            order_type: OrderType::Alo,
            limit_price: Some(limit_price),
            queue_fill_min_ppm: None,
            queue_fill_max_ppm: None,
        }
    }

    pub fn validate(self) -> Result<(), SimError> {
        match self.order_type {
            OrderType::Market => {
                if self.queue_fill_min_ppm.is_some() || self.queue_fill_max_ppm.is_some() {
                    return Err(SimError::InvalidRequest {
                        field: "order_policy.unexpected_queue",
                    });
                }
                Ok(())
            }
            OrderType::Ioc => {
                if self.queue_fill_min_ppm.is_some() || self.queue_fill_max_ppm.is_some() {
                    return Err(SimError::InvalidRequest {
                        field: "order_policy.unexpected_queue",
                    });
                }
                Ok(())
            }
            OrderType::Gtc => {
                if self.limit_price.is_none()
                    || self.queue_fill_min_ppm.is_none()
                    || self.queue_fill_max_ppm.is_none()
                {
                    return Err(SimError::UnmodeledCost {
                        component: "gtc_queue_or_limit",
                    });
                }
                Ok(())
            }
            OrderType::Alo => {
                if self.limit_price.is_none() {
                    return Err(SimError::UnmodeledCost {
                        component: "alo_limit",
                    });
                }
                Ok(())
            }
        }
    }

    #[must_use]
    pub const fn order_type(self) -> OrderType {
        self.order_type
    }

    #[must_use]
    pub const fn limit_price(self) -> Option<Price> {
        self.limit_price
    }

    pub fn queue_fill_ppm(self, seed: u64) -> Result<Option<u32>, SimError> {
        match (self.queue_fill_min_ppm, self.queue_fill_max_ppm) {
            (None, None) => Ok(None),
            (Some(min), Some(max)) => {
                if min > max || max > 1_000_000 {
                    return Err(SimError::UnmodeledCost {
                        component: "gtc_queue_model",
                    });
                }
                if min == max {
                    return Ok(Some(min));
                }
                let span = u64::from(max - min);
                let offset =
                    u32::try_from(seed % (span + 1)).map_err(|_| SimError::InvalidAmount)?;
                Ok(Some(min + offset))
            }
            _ => Err(SimError::UnmodeledCost {
                component: "gtc_queue_model",
            }),
        }
    }
}

#[must_use]
pub fn entry_is_buy(direction: Direction) -> bool {
    match direction {
        Direction::Long => true,
        Direction::Short => false,
        Direction::Flat => false,
    }
}
