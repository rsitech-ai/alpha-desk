use domain_types::{Address, MarketId, VaultId};
use storage_ports::{StateStoreError, admit_column_family_schema};

pub use storage_ports::{
    LEGACY_ROCKSDB_STATE_STORE_SCHEMA, STATE_STORE_CFS as COLUMN_FAMILIES,
    STATE_STORE_ENGINE as ENGINE_ID, STATE_STORE_SCHEMA as SCHEMA_ID,
};

// ponytail: CF names and length-prefixed keys for spec §19.3. Production engine is
// SyncedWriteBatchStore (file generations + HEAD). RocksDB 11.1 stays out: deny.toml
// has no GPL and librocksdb-sys is GPL-2.0. A later permitted engine gets its own SCHEMA id.
pub const CF_META: &str = "meta";
pub const CF_MARKET_STATE: &str = "market_state";
pub const CF_L2_BOOK: &str = "l2_book";
pub const CF_L4_ORDERS: &str = "l4_orders";
pub const CF_ACCOUNT_STATE: &str = "account_state";
pub const CF_BALANCES: &str = "balances";
pub const CF_POSITIONS: &str = "positions";
pub const CF_ORDERS: &str = "orders";
pub const CF_TWAP: &str = "twap";
pub const CF_VAULTS: &str = "vaults";
pub const CF_STAKING: &str = "staking";
pub const CF_BORROW_LEND: &str = "borrow_lend";
pub const CF_EVM_HEADS: &str = "evm_heads";
pub const CF_RECONCILIATION: &str = "reconciliation";
pub const CF_EVENT_SEEN: &str = "event_seen";
pub const CF_CHECKPOINTS: &str = "checkpoints";

const MAX_STATE_KEY_BYTES: usize = 64 * 1024;
const KEY_FRAME_BYTES: usize = size_of::<u64>();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEncodingError {
    EmptyPart,
    TooLarge,
}

impl KeyEncodingError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::EmptyPart => "state_store.key_empty_part",
            Self::TooLarge => "state_store.key_too_large",
        }
    }
}

pub fn framed_key(parts: &[&[u8]]) -> Result<Vec<u8>, KeyEncodingError> {
    let encoded_len = parts.iter().try_fold(0_usize, |total, part| {
        if part.is_empty() {
            return Err(KeyEncodingError::EmptyPart);
        }
        total
            .checked_add(KEY_FRAME_BYTES)
            .and_then(|length| length.checked_add(part.len()))
            .ok_or(KeyEncodingError::TooLarge)
    })?;
    if encoded_len == 0 || encoded_len > MAX_STATE_KEY_BYTES {
        return Err(KeyEncodingError::TooLarge);
    }
    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(encoded_len)
        .map_err(|_| KeyEncodingError::TooLarge)?;
    for part in parts {
        let length = u64::try_from(part.len()).map_err(|_| KeyEncodingError::TooLarge)?;
        encoded.extend_from_slice(&length.to_be_bytes());
        encoded.extend_from_slice(part);
    }
    Ok(encoded)
}

pub fn vault_current_key(vault_id: &VaultId) -> Result<Vec<u8>, KeyEncodingError> {
    framed_key(&[vault_id.as_str().as_bytes()])
}

pub fn staking_liquid_key(account_id: &Address) -> Result<Vec<u8>, KeyEncodingError> {
    framed_key(&[account_id.as_bytes()])
}

pub fn l4_order_key(market_id: &MarketId, order_id: &[u8]) -> Result<Vec<u8>, KeyEncodingError> {
    framed_key(&[market_id.as_str().as_bytes(), order_id])
}

pub fn l2_book_key(market_id: &MarketId) -> Result<Vec<u8>, KeyEncodingError> {
    framed_key(&[market_id.as_str().as_bytes()])
}

pub const NAMESPACE_COLUMN_FAMILIES: &[(&str, &str)] = &[
    ("account-fact.v1", CF_EVENT_SEEN),
    ("account-subaccount-master.v1", CF_ACCOUNT_STATE),
    ("account-vault-relation.v1", CF_ACCOUNT_STATE),
    ("account-quantity-flow-current.v1", CF_BALANCES),
    ("account-quote-flow-current.v1", CF_BALANCES),
    ("account-mode-current.v1", CF_ACCOUNT_STATE),
    ("account-margin-mode-current.v1", CF_ACCOUNT_STATE),
    ("account-leverage-current.v1", CF_ACCOUNT_STATE),
    ("vault-principal-flow-current.v1", CF_VAULTS),
    ("vault-share-flow-current.v1", CF_VAULTS),
    ("vault-fact.v1", CF_VAULTS),
    ("vault-current.v1", CF_VAULTS),
    ("staking-fact.v1", CF_STAKING),
    ("staking-liquid-current.v1", CF_STAKING),
    ("staking-pending-current.v1", CF_STAKING),
    ("staking-delegation-current.v1", CF_STAKING),
    ("staking-delegation-relation.v1", CF_STAKING),
    ("validator-fact.v1", CF_STAKING),
    ("validator-reward-current.v1", CF_STAKING),
    ("market-fact.v1", CF_MARKET_STATE),
    ("dex-current.v1", CF_MARKET_STATE),
    ("asset-context-current.v1", CF_MARKET_STATE),
    ("market-current.v1", CF_MARKET_STATE),
    ("market-metadata-version.v1", CF_MARKET_STATE),
    ("market-outcome-current.v1", CF_MARKET_STATE),
    ("order-fact.v1", CF_ORDERS),
    ("order-current.v1", CF_ORDERS),
    ("order-transition.v1", CF_ORDERS),
    ("trigger-fact.v1", CF_ORDERS),
    ("trigger-current.v1", CF_ORDERS),
    ("trigger-transition.v1", CF_ORDERS),
    ("twap-fact.v1", CF_TWAP),
    ("twap-current.v1", CF_TWAP),
    ("twap-transition.v1", CF_TWAP),
    ("trade.v1", CF_EVENT_SEEN),
    ("trade-participant.v1", CF_EVENT_SEEN),
    ("trade.v2", CF_EVENT_SEEN),
    ("trade-participant.v2", CF_EVENT_SEEN),
    ("reconciliation.v1", CF_RECONCILIATION),
    ("trade-reconciliation.v2", CF_RECONCILIATION),
    ("position-quantity-current.v1", CF_POSITIONS),
    ("position-effect-fact.v1", CF_POSITIONS),
    ("position-unresolved-cause-fact.v1", CF_POSITIONS),
    ("position-episode.v1", CF_POSITIONS),
    ("position-episode-current.v1", CF_POSITIONS),
    ("position-episode-effect-fact.v1", CF_POSITIONS),
    ("liquidation-current.v1", CF_POSITIONS),
    ("liquidation-start-fact.v1", CF_POSITIONS),
    ("liquidation-fill-fact.v1", CF_POSITIONS),
    ("liquidation-market-flow-current.v1", CF_POSITIONS),
    ("backstop-liquidation-fact.v1", CF_POSITIONS),
    ("position-settlement-fact.v1", CF_POSITIONS),
];

#[must_use]
pub fn column_family_for_namespace(namespace: &str) -> Option<&'static str> {
    NAMESPACE_COLUMN_FAMILIES
        .iter()
        .find(|(name, _)| *name == namespace)
        .map(|(_, column_family)| *column_family)
}

pub fn admit_schema(observed: &[&str]) -> Result<(), StateStoreError> {
    admit_column_family_schema(observed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use canonical_ledger::{StakingLiquidCurrentRecordV1, VaultCurrentRecordV1};
    use domain_types::{Address, MarketId, OrderId, VaultId};
    use storage_ports::STATE_STORE_CFS;

    #[test]
    fn catalog_matches_spec_19_3_and_refuses_in_place_reinterpretation() {
        assert_eq!(
            STATE_STORE_CFS,
            [
                "meta",
                "market_state",
                "l2_book",
                "l4_orders",
                "account_state",
                "balances",
                "positions",
                "orders",
                "twap",
                "vaults",
                "staking",
                "borrow_lend",
                "evm_heads",
                "reconciliation",
                "event_seen",
                "checkpoints",
            ]
        );
        assert_eq!(CF_VAULTS, "vaults");
        assert_eq!(CF_STAKING, "staking");
        assert_eq!(CF_BORROW_LEND, "borrow_lend");
        assert_eq!(CF_L4_ORDERS, "l4_orders");
        assert_eq!(CF_L2_BOOK, "l2_book");
        assert_eq!(CF_CHECKPOINTS, "checkpoints");
        admit_schema(STATE_STORE_CFS).unwrap();
        let error = admit_schema(&STATE_STORE_CFS[1..]).unwrap_err();
        assert_eq!(error.reason_code(), "state_store.rebuild_required");
        assert_eq!(
            SCHEMA_ID,
            "hyperliquid-alpha-desk/file-atomic-state-store/v1"
        );
        assert_eq!(ENGINE_ID, "file-atomic");
        assert_ne!(SCHEMA_ID, LEGACY_ROCKSDB_STATE_STORE_SCHEMA);
        assert!(!SCHEMA_ID.contains("rocksdb"));
    }

    #[test]
    fn framed_key_matches_ledger_identity_bytes_and_rejects_empty_parts() {
        let vault = VaultId::new("vault-a").unwrap();
        let account = Address::from_bytes([0x11; 20]);
        let market = MarketId::new("perp:BTC").unwrap();
        let order = OrderId::new("oid-1").unwrap();
        let vault_key = VaultCurrentRecordV1::state_key(&vault).unwrap();
        let staking_key = StakingLiquidCurrentRecordV1::state_key(&account).unwrap();
        assert_eq!(vault_key.namespace(), "vault-current.v1");
        assert_eq!(vault_key.key(), vault_current_key(&vault).unwrap());
        assert_eq!(
            column_family_for_namespace(vault_key.namespace()),
            Some(CF_VAULTS)
        );
        assert_eq!(staking_key.namespace(), "staking-liquid-current.v1");
        assert_eq!(staking_key.key(), staking_liquid_key(&account).unwrap());
        assert_eq!(
            column_family_for_namespace(staking_key.namespace()),
            Some(CF_STAKING)
        );
        assert_eq!(
            l4_order_key(&market, order.as_str().as_bytes()).unwrap(),
            orderbook::l4_order_key(&market, &order)
        );
        assert_eq!(
            l2_book_key(&market).unwrap(),
            orderbook::l2_book_key(&market)
        );
        assert_eq!(
            framed_key(&[b""]).unwrap_err().reason_code(),
            "state_store.key_empty_part"
        );
        assert_eq!(
            framed_key(&[]).unwrap_err().reason_code(),
            "state_store.key_too_large"
        );
    }

    #[test]
    fn every_production_namespace_maps_to_a_catalog_column_family() {
        let mut seen = std::collections::BTreeSet::new();
        for (namespace, column_family) in NAMESPACE_COLUMN_FAMILIES {
            assert!(seen.insert(*namespace), "duplicate namespace {namespace}");
            assert!(
                STATE_STORE_CFS.contains(column_family),
                "namespace {namespace} maps to unknown CF {column_family}"
            );
            assert_eq!(column_family_for_namespace(namespace), Some(*column_family));
        }
        assert!(column_family_for_namespace("borrow-lend-current.v1").is_none());
        assert!(column_family_for_namespace("not-a-namespace").is_none());
        assert_eq!(NAMESPACE_COLUMN_FAMILIES.len(), 52);
    }
}
