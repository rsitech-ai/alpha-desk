use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use domain_types::{Address, Decimal};
use hl_protocol::evm::{
    BlockPace, CORE_WRITER_ADDRESS, CoreWriterAction, CoreWriterCall, EvmBlock,
    EvmBlockAndReceipts, EvmChainId, EvmError, EvmHeader, EvmLog, EvmReceipt, EvmTransaction,
    Hash32, NativeHypeTransfer, PrecompileObservation, ReceiptStatus, SystemTransaction,
    TRANSFER_TOPIC, TxKind, Wei, WellKnownLog, decode_rmp_lz4, encode_rmp_lz4, is_core_writer,
    is_read_precompile,
};
use serde_json::json;

fn hash32(byte: u8) -> Hash32 {
    Hash32::from_bytes([byte; 32])
}

fn address(byte: u8) -> Address {
    Address::from_bytes([byte; 20])
}

fn topic_address(byte: u8) -> Hash32 {
    let mut bytes = [0_u8; 32];
    bytes[12..].copy_from_slice(&[byte; 20]);
    Hash32::from_bytes(bytes)
}

fn extra_blob_salt() -> BTreeMap<String, serde_json::Value> {
    BTreeMap::from([("blobSalt".to_owned(), json!("0xdeadbeef"))])
}

fn sample_tx(
    chain: EvmChainId,
    block_hash: Hash32,
    block_number: u64,
    index: u32,
    hash: Hash32,
    extra: BTreeMap<String, serde_json::Value>,
) -> EvmTransaction {
    EvmTransaction::new(
        chain,
        block_hash,
        block_number,
        index,
        hash,
        "Eip1559",
        Some(address(0x11)),
        Some(address(0x22)),
        7,
        Wei::from_u64(21_000),
        None,
        Some(Wei::from_u64(1_000_000_000)),
        Some(Wei::ZERO),
        Wei::from_u64(1_000_000_000_000_000_000),
        vec![0x01, 0x02],
        vec![vec![1, 2, 3]],
        extra,
    )
    .expect("tx")
}

fn sample_record(
    chain: EvmChainId,
    number: u64,
    hash: Hash32,
    parent: Hash32,
) -> EvmBlockAndReceipts {
    let header = EvmHeader::new(hash, parent, number, 1_700_000_000, address(0xaa));
    let block = EvmBlock::new(chain, header, BlockPace::Unknown);
    let tx = sample_tx(chain, hash, number, 0, hash32(0x44), extra_blob_salt());
    let log = EvmLog::new(
        chain,
        hash,
        number,
        tx.hash(),
        0,
        0,
        address(0xee),
        vec![TRANSFER_TOPIC, topic_address(0x11), topic_address(0x22)],
        Wei::from_u64(5).as_be_bytes().to_vec(),
    );
    let receipt = EvmReceipt::new(
        chain,
        hash,
        number,
        tx.hash(),
        0,
        ReceiptStatus::Success,
        Some(Wei::from_u64(21_000)),
        Wei::from_u64(21_000),
        None,
        vec![log],
    );
    let system = SystemTransaction::from_transaction(
        EvmTransaction::new(
            chain,
            hash,
            number,
            0,
            hash32(0x55),
            "Legacy",
            None,
            Some(address(0x33)),
            0,
            Wei::from_u64(21_000),
            Some(Wei::from_u64(1)),
            None,
            None,
            Wei::from_u64(3),
            Vec::new(),
            Vec::new(),
            BTreeMap::new(),
        )
        .expect("system tx"),
        None,
    )
    .expect("system");
    EvmBlockAndReceipts::new(block, vec![tx], vec![receipt], vec![system]).expect("record")
}

fn parquet_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/parquet")
}

mod evm {
    use super::*;

    #[test]
    fn msgpack_lz4_fixture_decodes_without_rpc() {
        let mainnet = sample_record(EvmChainId::MAINNET, 6123, hash32(0x61), hash32(0x60));
        let bytes = encode_rmp_lz4(&[mainnet]).expect("encode");
        assert_eq!(&bytes[..4], &[0x04, 0x22, 0x4d, 0x18]);
        let decoded = decode_rmp_lz4(&bytes, EvmChainId::MAINNET).expect("decode");
        assert_eq!(decoded.len(), 1);
        let record = &decoded[0];
        assert_eq!(record.block().chain_id(), EvmChainId::MAINNET);
        assert_eq!(record.block().number(), 6123);
        assert_eq!(record.block().hash(), hash32(0x61));
        assert_eq!(record.block().parent_hash(), hash32(0x60));
        assert_eq!(record.block().pace(), BlockPace::Unknown);
        assert_eq!(record.transactions()[0].kind(), Some(TxKind::Eip1559));
        assert_eq!(
            record.transactions()[0].extra().get("blobSalt"),
            Some(&json!("0xdeadbeef"))
        );
        assert_eq!(record.receipts()[0].status(), ReceiptStatus::Success);
        assert_eq!(record.logs().count(), 1);
        assert_eq!(record.system_transactions().len(), 1);
        assert!(
            record.system_transactions()[0]
                .transaction()
                .unsigned_system_candidate()
        );
    }

    #[test]
    fn testnet_and_mainnet_chain_ids_round_trip() {
        for (chain, number) in [
            (EvmChainId::MAINNET, 6123_u64),
            (EvmChainId::TESTNET, 18_000_000),
        ] {
            let record = sample_record(chain, number, hash32(0x10), hash32(0x0f));
            let decoded = decode_rmp_lz4(&encode_rmp_lz4(&[record]).unwrap(), chain).unwrap();
            assert_eq!(decoded[0].block().chain_id(), chain);
            assert_eq!(decoded[0].block().number(), number);
        }
        assert!(matches!(
            EvmChainId::new(1),
            Err(EvmError::UnsupportedChainId(1))
        ));
    }

    #[test]
    fn block_parent_identity_is_hash_and_number() {
        let parent = EvmBlock::new(
            EvmChainId::MAINNET,
            EvmHeader::new(hash32(0x01), hash32(0x00), 6122, 1, address(0xaa)),
            BlockPace::Unknown,
        );
        let child = EvmBlock::new(
            EvmChainId::MAINNET,
            EvmHeader::new(hash32(0x02), hash32(0x01), 6123, 2, address(0xaa)),
            BlockPace::Unknown,
        );
        assert!(parent.is_parent_of(&child));
        assert!(!child.is_parent_of(&parent));
    }

    #[test]
    fn transaction_and_log_ids_are_stable() {
        let record = sample_record(EvmChainId::MAINNET, 1, hash32(0xab), hash32(0xaa));
        let tx = &record.transactions()[0];
        let log = &record.receipts()[0].logs()[0];
        assert!(tx.tx_id().starts_with("0x"));
        assert!(tx.fact_id().starts_with("evx_"));
        assert_eq!(
            log.id().as_wire(),
            format!("{}:0", tx.hash().to_api_string())
        );
        assert!(log.fact_id().starts_with("evl_"));
        let again = sample_record(EvmChainId::MAINNET, 1, hash32(0xab), hash32(0xaa));
        assert_eq!(tx.fact_hash(), again.transactions()[0].fact_hash());
        assert_eq!(log.fact_hash(), again.receipts()[0].logs()[0].fact_hash());
        assert_eq!(record.block().fact_hash(), again.block().fact_hash());
    }

    #[test]
    fn wei_unit_conversion_is_eighteen_decimal_hype() {
        let one = Wei::from_hype_decimal("1".parse::<Decimal>().unwrap()).unwrap();
        assert_eq!(one.to_decimal_string(), "1000000000000000000");
        assert_eq!(
            one.to_hype_decimal().unwrap().to_string(),
            "1.000000000000000000"
        );
        let wei = Wei::from_u64(1);
        assert_eq!(
            wei.to_hype_decimal().unwrap().to_string(),
            "0.000000000000000001"
        );
        assert_eq!(
            Wei::from_be_bytes([0xff; 32]).to_hype_decimal(),
            Err(EvmError::QuantityOverflow)
        );
    }

    #[test]
    fn receipt_status_success_and_failure() {
        let success = EvmReceipt::new(
            EvmChainId::MAINNET,
            hash32(1),
            1,
            hash32(2),
            0,
            ReceiptStatus::Success,
            None,
            Wei::from_u64(1),
            None,
            Vec::new(),
        );
        let failure = EvmReceipt::new(
            EvmChainId::MAINNET,
            hash32(1),
            1,
            hash32(2),
            0,
            ReceiptStatus::Failure,
            None,
            Wei::from_u64(1),
            None,
            Vec::new(),
        );
        assert_eq!(success.status().as_wire_name(), "success");
        assert_eq!(failure.status().as_wire_name(), "failure");
    }

    #[test]
    fn unknown_typed_transaction_fields_are_preserved() {
        let tx = sample_tx(
            EvmChainId::TESTNET,
            hash32(0x01),
            18_000_000,
            0,
            hash32(0x02),
            extra_blob_salt(),
        );
        assert_eq!(tx.kind(), Some(TxKind::Eip1559));
        assert_eq!(tx.extra().get("blobSalt"), Some(&json!("0xdeadbeef")));
        let record = sample_record(EvmChainId::TESTNET, 18_000_000, hash32(0x01), hash32(0x00));
        let decoded =
            decode_rmp_lz4(&encode_rmp_lz4(&[record]).unwrap(), EvmChainId::TESTNET).unwrap();
        assert_eq!(
            decoded[0].transactions()[0].extra().get("blobSalt"),
            Some(&json!("0xdeadbeef"))
        );
    }

    #[test]
    fn system_transactions_are_first_class_chain_facts() {
        let record = sample_record(EvmChainId::MAINNET, 9, hash32(0x09), hash32(0x08));
        let system = &record.system_transactions()[0];
        assert_eq!(system.transaction().type_name(), "Legacy");
        assert!(system.origin().is_none());
        assert!(system.transaction().unsigned_system_candidate());
    }

    #[test]
    fn native_transfer_and_erc20_log_decode() {
        let record = sample_record(EvmChainId::MAINNET, 3, hash32(0x03), hash32(0x02));
        let native = NativeHypeTransfer::from_transaction(&record.transactions()[0]).unwrap();
        assert_eq!(native.value(), Wei::from_u64(1_000_000_000_000_000_000));
        let known = WellKnownLog::from_log(&record.receipts()[0].logs()[0]).unwrap();
        match known {
            WellKnownLog::Erc20Transfer(transfer) => {
                assert_eq!(transfer.from(), address(0x11));
                assert_eq!(transfer.to(), address(0x22));
                assert_eq!(transfer.value(), Wei::from_u64(5));
            }
            other => panic!("expected erc20 transfer, got {other:?}"),
        }
    }

    #[test]
    fn core_writer_and_read_precompile_constants() {
        assert!(is_core_writer(CORE_WRITER_ADDRESS));
        assert!(!is_read_precompile(CORE_WRITER_ADDRESS));
        let mut precompile = [0_u8; 20];
        precompile[18] = 0x08;
        precompile[19] = 0x07;
        let precompile = Address::from_bytes(precompile);
        assert!(is_read_precompile(precompile));
        let action = CoreWriterAction::parse(&[1, 0, 0, 7, 0xaa]).unwrap();
        assert_eq!(action.version(), 1);
        assert_eq!(action.action_id(), 7);
        assert_eq!(action.payload(), &[0xaa]);
        let call = CoreWriterCall::new(hash32(1), hash32(2), &[1, 0, 0, 1]).unwrap();
        assert_eq!(call.action().action_id(), 1);
        let observation =
            PrecompileObservation::new(hash32(1), precompile, vec![0; 32], vec![0; 32]).unwrap();
        assert_eq!(observation.precompile_offset(), 0x0807);
    }

    #[test]
    fn parquet_schemas_use_numeric_chain_id() {
        for name in [
            "evm-blocks-v1.json",
            "evm-transactions-v1.json",
            "evm-receipts-v1.json",
            "evm-logs-v1.json",
        ] {
            let text = fs::read_to_string(parquet_root().join(name)).expect(name);
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["format"], "hyperliquid-alpha-desk/parquet-schema/v1");
            assert_eq!(value["fields"][0]["name"], "chain_id");
            assert_eq!(value["fields"][0]["type"], "uint64");
            assert!(!text.contains("canonical_event"));
        }
    }
}
