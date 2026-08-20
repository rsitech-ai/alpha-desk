use domain_types::{KnownTime, ProtocolTime, ValueError};

use crate::error::SimError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimClock {
    protocol_time: ProtocolTime,
    known_time: KnownTime,
}

impl SimClock {
    pub fn new(protocol_time: ProtocolTime, known_time: KnownTime) -> Result<Self, SimError> {
        if known_time < KnownTime::from_unix_micros(protocol_time.unix_micros())? {
            return Err(SimError::InvalidRequest {
                field: "clock.known_before_protocol",
            });
        }
        Ok(Self {
            protocol_time,
            known_time,
        })
    }

    #[must_use]
    pub const fn protocol_time(self) -> ProtocolTime {
        self.protocol_time
    }

    #[must_use]
    pub const fn known_time(self) -> KnownTime {
        self.known_time
    }

    pub fn advance(&mut self, delay_micros: u64) -> Result<(), SimError> {
        self.protocol_time = add_protocol(self.protocol_time, delay_micros)?;
        self.known_time = add_known(self.known_time, delay_micros)?;
        Ok(())
    }
}

pub(crate) fn add_protocol(
    time: ProtocolTime,
    delay_micros: u64,
) -> Result<ProtocolTime, SimError> {
    let next = time
        .unix_micros()
        .checked_add(i64::try_from(delay_micros).map_err(|_| SimError::InvalidAmount)?)
        .ok_or(SimError::InvalidAmount)?;
    ProtocolTime::from_unix_micros(next).map_err(SimError::from)
}

pub(crate) fn add_known(time: KnownTime, delay_micros: u64) -> Result<KnownTime, SimError> {
    let next = time
        .unix_micros()
        .checked_add(i64::try_from(delay_micros).map_err(|_| SimError::InvalidAmount)?)
        .ok_or(SimError::InvalidAmount)?;
    KnownTime::from_unix_micros(next).map_err(SimError::from)
}

pub(crate) fn protocol_as_known(time: ProtocolTime) -> Result<KnownTime, ValueError> {
    KnownTime::from_unix_micros(time.unix_micros())
}
