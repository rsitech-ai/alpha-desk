#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use bytes::Bytes;
use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use domain_types::{
    BlockHeight, BlockRange, ChainId, KnownTime, Price, ProtocolTime, Quantity, SourceId,
    TransactionId,
};
use hl_analytics::archive::{ArchiveConfig, LocalParquetArchive};
use hl_protocol::{
    ObservationClass, ParseWarning, ReceiveTimestamps, SourceCursor, SourceObservation,
};
use storage_ports::{
    CanonicalArchive, CanonicalArchiveMaintenance, RawObservationArchive, RawObservationBatch,
};

const FIXED_TIME_MICROS: i64 = 1_721_779_300_000_000;

fn main() -> Result<(), Box<dyn Error>> {
    let output = parse_output()?;
    prepare_empty_directory(&output)?;
    let archive = LocalParquetArchive::open(
        &output,
        ArchiveConfig::deterministic_fixture(
            "archive-fixture-v1",
            KnownTime::from_unix_micros(FIXED_TIME_MICROS)?,
        )?,
    )?;

    for (height, seed, events) in [(700_u64, 70_u64, 2_usize), (701, 71, 0), (702, 72, 1)] {
        archive.append_block(&canonical_block(height, seed, events)?)?;
    }
    archive.compact_range(
        &ChainId::new("mainnet")?,
        BlockRange::new(BlockHeight::new(700), BlockHeight::new(702))?,
    )?;
    archive.append_batch(&raw_batch()?)?;

    let inspection = archive.inspect()?;
    if inspection.canonical_chains() != 1
        || inspection.raw_sources() != 1
        || inspection.canonical_blocks() != 3
        || inspection.canonical_events() != 3
        || inspection.raw_observations() != 3
        || inspection.objects().len() != 2
    {
        return Err(
            "generated archive inspection did not match the frozen fixture contract".into(),
        );
    }
    println!(
        "fixture:ok chains=1 raw_sources=1 blocks=3 canonical_events=3 raw_observations=3 objects=2"
    );
    Ok(())
}

fn parse_output() -> Result<PathBuf, Box<dyn Error>> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let output = arguments
        .next()
        .ok_or("usage: generate-fixture <empty-output-directory>")?;
    if arguments.next().is_some() {
        return Err("usage: generate-fixture <empty-output-directory>".into());
    }
    Ok(PathBuf::from(output))
}

fn prepare_empty_directory(path: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(path)?;
    if fs::read_dir(path)?.next().is_some() {
        return Err("fixture output directory must be empty".into());
    }
    Ok(())
}

fn canonical_block(
    height: u64,
    payload_seed: u64,
    event_count: usize,
) -> Result<BlockEnvelope, Box<dyn Error>> {
    let block_time_micros = 1_721_779_200_000_000_i64
        .checked_add(i64::try_from(height)?)
        .ok_or("fixture block time overflow")?;
    let block_time = ProtocolTime::from_unix_micros(block_time_micros)?;
    let chain = ChainId::new("mainnet")?;
    let source = SourceId::new("primary-node")?;
    let events = (0..event_count)
        .map(|index| {
            let index = u32::try_from(index)?;
            Ok(CanonicalEventEnvelope::from_input(CanonicalEventInput {
                schema_version: "1.0.0".to_owned(),
                chain_id: chain.clone(),
                block_height: BlockHeight::new(height),
                block_time,
                transaction_id: TransactionId::new(format!("tx-{height}"))?,
                transaction_index: 0,
                canonical_event_index: index,
                market_ids: Vec::new(),
                account_ids: Vec::new(),
                source_evidence: vec![SourceEvidence::try_new(
                    source.clone(),
                    "node-v1",
                    format!("block-{height}:{index}"),
                    [u8::try_from(payload_seed)?; 32],
                )?],
                confirmation_class: ConfirmationClass::CommittedPrimary,
                observed_at: KnownTime::from_unix_micros(block_time_micros)?,
                ingested_at: KnownTime::from_unix_micros(block_time_micros + 1)?,
                canonicalized_at: KnownTime::from_unix_micros(block_time_micros + 2)?,
                parser_version: "canonical-parser-v1".to_owned(),
                payload: EventPayload::TradeMatched(TradeMatched::without_identities(
                    Price::parse_at_scale("65000", 6)?,
                    Quantity::parse_at_scale("0.01", 8)?,
                    payload_seed + u64::from(index),
                )),
            })?)
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    Ok(BlockEnvelope::try_new(
        chain,
        BlockHeight::new(height),
        block_time,
        ConfirmationClass::CommittedPrimary,
        events,
        BTreeMap::from([(source, [0x55; 32])]),
    )?)
}

fn raw_batch() -> Result<RawObservationBatch, Box<dyn Error>> {
    let observations = [b"raw-700".as_slice(), b"raw-701", b"raw-702"]
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            let offset = 700_u64 + u64::try_from(index)?;
            Ok(SourceObservation::new(
                SourceId::new("primary-node")?,
                "node-v1",
                ObservationClass::CommittedBlock,
                SourceCursor::new("node-session-fixture", offset)?,
                ReceiveTimestamps::new(
                    1_721_779_200_000_000_i64 + i64::try_from(offset)?,
                    9_000_000 + offset,
                )?,
                "raw-parser-v1",
                Bytes::copy_from_slice(payload),
                vec![ParseWarning::new(
                    "fixture-warning",
                    format!("offset={offset}"),
                )?],
                1024,
            )?)
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    Ok(RawObservationBatch::try_new(
        ChainId::new("mainnet")?,
        observations,
        [0xa1; 32],
        [0xb2; 32],
    )?)
}
