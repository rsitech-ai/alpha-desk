use domain_types::{Address, MarketId, VaultId};
use storage_ports::{StateStoreError, admit_column_family_schema};

pub use storage_ports::{STATE_STORE_CFS as COLUMN_FAMILIES, STATE_STORE_SCHEMA as SCHEMA_ID};

// ponytail: CF names and length-prefixed keys for spec §19.3. RocksDB 11.1 stays out: deny.toml blocks librocksdb-sys (GPL-2.0) and crates.io rust-rocksdb wraps ~10.x. Engine leftover is T19.
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

pub fn framed_key(parts: &[&[u8]]) -> Vec<u8> {
    let mut encoded = Vec::new();
    for part in parts {
        encoded.extend_from_slice(&(part.len() as u64).to_be_bytes());
        encoded.extend_from_slice(part);
    }
    encoded
}

pub fn vault_current_key(vault_id: &VaultId) -> Vec<u8> {
    framed_key(&[vault_id.as_str().as_bytes()])
}

pub fn staking_liquid_key(account_id: &Address) -> Vec<u8> {
    framed_key(&[account_id.as_bytes()])
}

pub fn l4_order_key(market_id: &MarketId, order_id: &[u8]) -> Vec<u8> {
    framed_key(&[market_id.as_str().as_bytes(), order_id])
}

pub fn admit_schema(observed: &[&str]) -> Result<(), StateStoreError> {
    admit_column_family_schema(observed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain_types::{Address, MarketId, VaultId};
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
    }

    #[test]
    fn framed_keys_round_trip_component_identities() {
        let vault = VaultId::new("vault-a").unwrap();
        let account = Address::from_bytes([0x11; 20]);
        let market = MarketId::new("perp:BTC").unwrap();
        assert_eq!(
            vault_current_key(&vault),
            framed_key(&[vault.as_str().as_bytes()])
        );
        assert_eq!(
            staking_liquid_key(&account),
            framed_key(&[account.as_bytes()])
        );
        assert_eq!(
            l4_order_key(&market, b"order-1"),
            framed_key(&[market.as_str().as_bytes(), b"order-1"])
        );
    }
}
