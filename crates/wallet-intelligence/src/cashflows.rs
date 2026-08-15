use domain_types::{ProtocolTime, UsdAmount};
use serde::{Deserialize, Serialize};

use crate::IntelligenceError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CashFlowKind {
    Deposit,
    Withdrawal,
}

impl CashFlowKind {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Deposit => "deposit",
            Self::Withdrawal => "withdrawal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalCashFlow {
    pub protocol_time: ProtocolTime,
    pub kind: CashFlowKind,
    pub amount: UsdAmount,
}

impl ExternalCashFlow {
    pub fn try_new(
        protocol_time: ProtocolTime,
        kind: CashFlowKind,
        amount: UsdAmount,
    ) -> Result<Self, IntelligenceError> {
        if amount.raw() <= 0 {
            return Err(IntelligenceError::Malformed {
                what: "cash_flow",
                reason: "amount must be strictly positive",
            });
        }
        Ok(Self {
            protocol_time,
            kind,
            amount,
        })
    }

    pub fn signed_amount(&self) -> Result<UsdAmount, IntelligenceError> {
        match self.kind {
            CashFlowKind::Deposit => Ok(self.amount),
            CashFlowKind::Withdrawal => {
                let negated = self
                    .amount
                    .raw()
                    .checked_neg()
                    .ok_or(IntelligenceError::Overflow)?;
                UsdAmount::from_raw(negated, self.amount.scale()).map_err(Into::into)
            }
        }
    }
}
