use domain_types::{AccountId, EntityId, MasterAccountId, VaultId};
use serde::{Deserialize, Serialize};

use crate::GraphError;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum GraphNodeId {
    Account(AccountId),
    MasterAccount(MasterAccountId),
    Vault(VaultId),
    Entity(EntityId),
}

impl GraphNodeId {
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Account(_) => "account",
            Self::MasterAccount(_) => "master_account",
            Self::Vault(_) => "vault",
            Self::Entity(_) => "entity",
        }
    }

    pub fn try_account(self) -> Result<AccountId, GraphError> {
        match self {
            Self::Account(account) => Ok(account),
            Self::MasterAccount(_) | Self::Vault(_) | Self::Entity(_) => {
                Err(GraphError::Malformed {
                    what: "graph_node",
                    reason: "expected account node",
                })
            }
        }
    }
}
