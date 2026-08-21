use super::wire::{self, WireValue};
use super::{EvmChainId, EvmError, HashProvenance, decode_rmp_lz4};

fn h32(byte: u8) -> WireValue {
    wire::bin_bytes(&[byte; 32])
}

fn addr(byte: u8) -> WireValue {
    wire::bin_bytes(&[byte; 20])
}

enum ChainIdField {
    Absent,
    Nil,
    Present(u64),
}

enum ReceiptsField {
    Omit,
    Items(Vec<WireValue>),
}

fn header(number: Option<u64>, timestamp: Option<u64>) -> WireValue {
    let mut pairs = vec![
        ("parentHash", h32(0x60)),
        ("sha3Uncles", h32(0x00)),
        ("miner", addr(0xaa)),
        ("stateRoot", h32(0x01)),
        ("transactionsRoot", h32(0x02)),
        ("receiptsRoot", h32(0x03)),
        ("extraData", wire::bin_bytes(&[])),
        ("gasLimit", WireValue::Uint(30_000_000)),
        ("gasUsed", WireValue::Uint(0)),
    ];
    if let Some(number) = number {
        pairs.push(("number", WireValue::Uint(number)));
    }
    if let Some(timestamp) = timestamp {
        pairs.push(("timestamp", WireValue::Uint(timestamp)));
    }
    wire::string_map(pairs)
}

fn legacy_tx(chain: ChainIdField, hash: Option<u8>) -> WireValue {
    let mut content = vec![
        ("nonce", WireValue::Uint(0)),
        ("gas", WireValue::Uint(21_000)),
        ("to", addr(0x22)),
        ("value", WireValue::Uint(1)),
        ("input", wire::bin_bytes(&[])),
    ];
    match chain {
        ChainIdField::Absent => {}
        ChainIdField::Nil => content.push(("chainId", WireValue::Nil)),
        ChainIdField::Present(id) => content.push(("chainId", WireValue::Uint(id))),
    }
    let tagged = WireValue::Map(vec![(
        WireValue::String("Legacy".to_owned()),
        wire::string_map(content),
    )]);
    let mut root = vec![
        ("transaction", tagged),
        ("signature", WireValue::Array(vec![])),
    ];
    if let Some(byte) = hash {
        root.push(("hash", h32(byte)));
    }
    wire::string_map(root)
}

fn ok_receipt() -> WireValue {
    wire::string_map(vec![
        ("status", WireValue::Uint(1)),
        ("cumulativeGasUsed", WireValue::Uint(21_000)),
        ("logs", WireValue::Array(vec![])),
    ])
}

fn receipt_with_tx_hash(byte: u8) -> WireValue {
    wire::string_map(vec![
        ("status", WireValue::Uint(1)),
        ("cumulativeGasUsed", WireValue::Uint(21_000)),
        ("logs", WireValue::Array(vec![])),
        ("transactionHash", h32(byte)),
    ])
}

fn receipt_with_tx_index(index: u64) -> WireValue {
    wire::string_map(vec![
        ("status", WireValue::Uint(1)),
        ("cumulativeGasUsed", WireValue::Uint(21_000)),
        ("logs", WireValue::Array(vec![])),
        ("transactionIndex", WireValue::Uint(index)),
    ])
}

fn pack(
    number: Option<u64>,
    timestamp: Option<u64>,
    txs: Vec<WireValue>,
    receipts: ReceiptsField,
    system: Vec<WireValue>,
) -> Vec<u8> {
    let sealed = wire::string_map(vec![
        ("hash", h32(0x61)),
        ("header", header(number, timestamp)),
    ]);
    let body = wire::string_map(vec![("transactions", WireValue::Array(txs))]);
    let reth = wire::string_map(vec![("header", sealed), ("body", body)]);
    let block = wire::string_map(vec![("Reth115", reth)]);
    let mut root = vec![("block", block)];
    match receipts {
        ReceiptsField::Omit => {}
        ReceiptsField::Items(items) => root.push(("receipts", WireValue::Array(items))),
    }
    if !system.is_empty() {
        root.push(("systemTransactions", WireValue::Array(system)));
    }
    let packed = wire::encode_msgpack(&wire::string_map(root)).expect("msgpack");
    wire::compress_lz4_frame(&packed).expect("lz4")
}

fn decode(bytes: &[u8]) -> Result<Vec<super::EvmBlockAndReceipts>, EvmError> {
    decode_rmp_lz4(bytes, EvmChainId::MAINNET)
}

fn one_tx_archive(chain: ChainIdField, hash: Option<u8>, receipts: ReceiptsField) -> Vec<u8> {
    pack(
        Some(6123),
        Some(1_700_000_000),
        vec![legacy_tx(chain, hash)],
        receipts,
        Vec::new(),
    )
}

#[test]
fn absent_tx_chain_id_uses_archive_fallback() {
    let bytes = one_tx_archive(
        ChainIdField::Absent,
        Some(0x44),
        ReceiptsField::Items(vec![ok_receipt()]),
    );
    let decoded = decode(&bytes).expect("legacy tx without chainId");
    assert_eq!(decoded[0].block().chain_id(), EvmChainId::MAINNET);
    assert_eq!(decoded[0].transactions()[0].chain_id(), EvmChainId::MAINNET);
}

#[test]
fn nil_tx_chain_id_uses_archive_fallback() {
    let bytes = one_tx_archive(
        ChainIdField::Nil,
        Some(0x44),
        ReceiptsField::Items(vec![ok_receipt()]),
    );
    let decoded = decode(&bytes).expect("nil chainId");
    assert_eq!(decoded[0].transactions()[0].chain_id(), EvmChainId::MAINNET);
}

#[test]
fn present_zero_tx_chain_id_is_rejected() {
    let bytes = one_tx_archive(
        ChainIdField::Present(0),
        Some(0x44),
        ReceiptsField::Items(vec![ok_receipt()]),
    );
    assert_eq!(decode(&bytes).unwrap_err(), EvmError::UnsupportedChainId(0));
}

#[test]
fn present_unsupported_tx_chain_id_is_rejected() {
    let bytes = one_tx_archive(
        ChainIdField::Present(1),
        Some(0x44),
        ReceiptsField::Items(vec![ok_receipt()]),
    );
    assert_eq!(decode(&bytes).unwrap_err(), EvmError::UnsupportedChainId(1));
}

#[test]
fn present_mismatched_tx_chain_id_is_schema_drift() {
    let bytes = one_tx_archive(
        ChainIdField::Present(998),
        Some(0x44),
        ReceiptsField::Items(vec![ok_receipt()]),
    );
    match decode(&bytes).unwrap_err() {
        EvmError::SchemaDrift(detail) => {
            assert!(detail.contains("998"), "{detail}");
            assert!(detail.contains("999"), "{detail}");
        }
        other => panic!("expected schema drift, got {other:?}"),
    }
}

#[test]
fn system_tx_without_chain_id_uses_archive_fallback() {
    let bytes = pack(
        Some(6123),
        Some(1_700_000_000),
        Vec::new(),
        ReceiptsField::Omit,
        vec![legacy_tx(ChainIdField::Absent, Some(0x55))],
    );
    let decoded = decode(&bytes).expect("system tx without chainId");
    assert_eq!(
        decoded[0].system_transactions()[0].transaction().chain_id(),
        EvmChainId::MAINNET
    );
}

#[test]
fn missing_receipts_key_with_transactions_fails() {
    let bytes = one_tx_archive(ChainIdField::Present(999), Some(0x44), ReceiptsField::Omit);
    match decode(&bytes).unwrap_err() {
        EvmError::SchemaDrift(detail) => {
            assert!(detail.contains("receipts"), "{detail}");
        }
        other => panic!("expected schema drift, got {other:?}"),
    }
}

#[test]
fn short_receipts_array_fails() {
    let bytes = one_tx_archive(
        ChainIdField::Present(999),
        Some(0x44),
        ReceiptsField::Items(Vec::new()),
    );
    match decode(&bytes).unwrap_err() {
        EvmError::SchemaDrift(detail) => {
            assert!(detail.contains("receipt count"), "{detail}");
        }
        other => panic!("expected schema drift, got {other:?}"),
    }
}

#[test]
fn long_receipts_array_fails() {
    let bytes = one_tx_archive(
        ChainIdField::Present(999),
        Some(0x44),
        ReceiptsField::Items(vec![ok_receipt(), ok_receipt()]),
    );
    match decode(&bytes).unwrap_err() {
        EvmError::SchemaDrift(detail) => {
            assert!(detail.contains("receipt count"), "{detail}");
        }
        other => panic!("expected schema drift, got {other:?}"),
    }
}

#[test]
fn empty_block_may_omit_receipts() {
    let bytes = pack(
        Some(6123),
        Some(1_700_000_000),
        Vec::new(),
        ReceiptsField::Omit,
        Vec::new(),
    );
    let decoded = decode(&bytes).expect("empty body");
    assert!(decoded[0].transactions().is_empty());
    assert!(decoded[0].receipts().is_empty());
}

#[test]
fn receipt_transaction_hash_mismatch_fails() {
    let bytes = one_tx_archive(
        ChainIdField::Present(999),
        Some(0x44),
        ReceiptsField::Items(vec![receipt_with_tx_hash(0x99)]),
    );
    match decode(&bytes).unwrap_err() {
        EvmError::SchemaDrift(detail) => {
            assert!(detail.contains("transaction hash"), "{detail}");
        }
        other => panic!("expected schema drift, got {other:?}"),
    }
}

#[test]
fn receipt_transaction_index_mismatch_fails() {
    let bytes = one_tx_archive(
        ChainIdField::Present(999),
        Some(0x44),
        ReceiptsField::Items(vec![receipt_with_tx_index(7)]),
    );
    match decode(&bytes).unwrap_err() {
        EvmError::SchemaDrift(detail) => {
            assert!(detail.contains("transaction index"), "{detail}");
        }
        other => panic!("expected schema drift, got {other:?}"),
    }
}

#[test]
fn receipt_transaction_hash_match_decodes() {
    let bytes = one_tx_archive(
        ChainIdField::Present(999),
        Some(0x44),
        ReceiptsField::Items(vec![receipt_with_tx_hash(0x44)]),
    );
    let decoded = decode(&bytes).expect("matching hash");
    assert_eq!(decoded[0].receipts().len(), 1);
}

#[test]
fn receipt_transaction_index_match_decodes() {
    let bytes = one_tx_archive(
        ChainIdField::Present(999),
        Some(0x44),
        ReceiptsField::Items(vec![receipt_with_tx_index(0)]),
    );
    let decoded = decode(&bytes).expect("matching index");
    assert_eq!(decoded[0].receipts()[0].tx_index(), 0);
}

#[test]
fn missing_header_number_fails() {
    let bytes = pack(
        None,
        Some(1_700_000_000),
        vec![legacy_tx(ChainIdField::Present(999), Some(0x44))],
        ReceiptsField::Items(vec![ok_receipt()]),
        Vec::new(),
    );
    match decode(&bytes).unwrap_err() {
        EvmError::SchemaDrift(detail) => {
            assert!(detail.contains("number"), "{detail}");
        }
        other => panic!("expected schema drift, got {other:?}"),
    }
}

#[test]
fn missing_header_timestamp_fails() {
    let bytes = pack(
        Some(6123),
        None,
        vec![legacy_tx(ChainIdField::Present(999), Some(0x44))],
        ReceiptsField::Items(vec![ok_receipt()]),
        Vec::new(),
    );
    match decode(&bytes).unwrap_err() {
        EvmError::SchemaDrift(detail) => {
            assert!(detail.contains("timestamp"), "{detail}");
        }
        other => panic!("expected schema drift, got {other:?}"),
    }
}

#[test]
fn header_number_zero_is_accepted() {
    let bytes = pack(
        Some(0),
        Some(1_700_000_000),
        vec![legacy_tx(ChainIdField::Present(999), Some(0x44))],
        ReceiptsField::Items(vec![ok_receipt()]),
        Vec::new(),
    );
    let decoded = decode(&bytes).expect("genesis number");
    assert_eq!(decoded[0].block().number(), 0);
}

#[test]
fn omitted_tx_hash_is_derived_not_observed() {
    let bytes = one_tx_archive(
        ChainIdField::Present(999),
        None,
        ReceiptsField::Items(vec![ok_receipt()]),
    );
    let decoded = decode(&bytes).expect("derived hash");
    let tx = &decoded[0].transactions()[0];
    assert_eq!(tx.hash_provenance(), HashProvenance::Derived);
    assert!(!tx.hash_is_observed());
    assert!(tx.tx_id().starts_with("0x"));
    assert_eq!(tx.tx_id().len(), 66);
    let again = decode(&bytes).expect("stable derived hash");
    assert_eq!(tx.hash(), again[0].transactions()[0].hash());
}

#[test]
fn present_tx_hash_is_observed() {
    let bytes = one_tx_archive(
        ChainIdField::Present(999),
        Some(0x44),
        ReceiptsField::Items(vec![ok_receipt()]),
    );
    let decoded = decode(&bytes).expect("observed hash");
    let tx = &decoded[0].transactions()[0];
    assert_eq!(tx.hash_provenance(), HashProvenance::Observed);
    assert!(tx.hash_is_observed());
    assert_eq!(tx.hash(), super::Hash32::from_bytes([0x44; 32]));
}
