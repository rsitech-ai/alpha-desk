use domain_types::{KnownTime, Price, ProtocolTime};
use serde::{Deserialize, Serialize};

use crate::book::{BookSnapshot, select_book};
use crate::clock::add_protocol;
use crate::error::SimError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitPolicy {
    time_hold_micros: u64,
    take_profit: Option<Price>,
    stop: Option<Price>,
}

impl ExitPolicy {
    pub fn time_hold(time_hold_micros: u64) -> Self {
        Self {
            time_hold_micros,
            take_profit: None,
            stop: None,
        }
    }

    pub fn new(
        time_hold_micros: u64,
        take_profit: Option<Price>,
        stop: Option<Price>,
    ) -> Result<Self, SimError> {
        Ok(Self {
            time_hold_micros,
            take_profit,
            stop,
        })
    }

    #[must_use]
    pub const fn time_hold_micros(self) -> u64 {
        self.time_hold_micros
    }

    pub fn choose_exit(
        self,
        books: &[BookSnapshot],
        opened_at: ProtocolTime,
        evaluation_known_at: KnownTime,
        long: bool,
        invalidate_at: Option<ProtocolTime>,
    ) -> Result<(ProtocolTime, &'static str), SimError> {
        let time_exit = add_protocol(opened_at, self.time_hold_micros)?;
        let mut chosen_at = time_exit;
        let mut reason = "time";
        if let Some(invalid_at) = invalidate_at
            && invalid_at > opened_at
            && invalid_at <= time_exit
        {
            chosen_at = invalid_at;
            reason = "evidence_invalidation";
        }
        for book in books {
            if book.known_at() > evaluation_known_at {
                return Err(SimError::FutureData {
                    field: "book.known_at",
                });
            }
            if book.effective_at() <= opened_at || book.effective_at() > time_exit {
                continue;
            }
            if let Some(take_profit) = self.take_profit {
                let hit = if long {
                    book.best_bid() >= take_profit
                } else {
                    book.best_ask() <= take_profit
                };
                if hit && book.effective_at() < chosen_at {
                    chosen_at = book.effective_at();
                    reason = "take_profit";
                }
            }
            if let Some(stop) = self.stop {
                let hit = if long {
                    book.best_bid() <= stop
                } else {
                    book.best_ask() >= stop
                };
                if hit && book.effective_at() < chosen_at {
                    chosen_at = book.effective_at();
                    reason = "stop";
                }
            }
        }
        select_book(books, chosen_at, evaluation_known_at)?;
        Ok((chosen_at, reason))
    }
}
