use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FailureInjection {
    None,
    RejectOrder,
    DelaySubmission { extra_micros: u64 },
    MarkBookStale,
    RemoveBookLiquidity,
}

impl FailureInjection {
    #[must_use]
    pub const fn extra_delay_micros(self) -> u64 {
        match self {
            Self::DelaySubmission { extra_micros } => extra_micros,
            Self::None | Self::RejectOrder | Self::MarkBookStale | Self::RemoveBookLiquidity => 0,
        }
    }
}
