use super::{EvmError, EvmTransaction};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreOrigin {
    core_height: u64,
    action_id: Option<String>,
}

impl CoreOrigin {
    pub fn new(core_height: u64, action_id: Option<String>) -> Result<Self, EvmError> {
        if let Some(action_id) = &action_id
            && (action_id.is_empty() || action_id.trim() != action_id)
        {
            return Err(EvmError::InvalidIdentity);
        }
        Ok(Self {
            core_height,
            action_id,
        })
    }

    #[must_use]
    pub const fn core_height(&self) -> u64 {
        self.core_height
    }

    #[must_use]
    pub fn action_id(&self) -> Option<&str> {
        self.action_id.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SystemTransaction {
    transaction: EvmTransaction,
    origin: Option<CoreOrigin>,
}

impl SystemTransaction {
    pub fn from_transaction(
        transaction: EvmTransaction,
        origin: Option<CoreOrigin>,
    ) -> Result<Self, EvmError> {
        Ok(Self {
            transaction,
            origin,
        })
    }

    #[must_use]
    pub const fn transaction(&self) -> &EvmTransaction {
        &self.transaction
    }

    #[must_use]
    pub const fn origin(&self) -> Option<&CoreOrigin> {
        self.origin.as_ref()
    }
}
