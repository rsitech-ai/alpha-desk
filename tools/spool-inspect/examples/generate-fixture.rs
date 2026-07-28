#![forbid(unsafe_code)]

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use domain_types::SourceId;
use hl_capture::spool::{DurabilityPolicy, SegmentHeaderV1, SpoolWriter};
use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor, SourceObservation};

const BUILD_HASH: [u8; 32] = [0x42; 32];

fn main() -> Result<(), Box<dyn Error>> {
    let output = parse_output()?;
    prepare_empty_directory(&output)?;
    let header = SegmentHeaderV1::new(
        SourceId::new("primary-node")?,
        "node-v1.2.3",
        "spool-v1",
        1,
        1_721_000_000_000_000,
        BUILD_HASH,
    )?;
    let mut writer = SpoolWriter::create(&output, header, DurabilityPolicy::FsyncEveryRecord)?;
    for (offset, payload) in [
        (40, Bytes::from_static(b"first")),
        (41, Bytes::from_static(b"second")),
        (42, Bytes::from_static(b"third")),
    ] {
        writer.append(
            &observation(offset, payload)?,
            1_721_000_000_000_100 + i64::try_from(offset)?,
        )?;
    }
    let receipt = writer.close(1_721_000_000_000_200, None)?;
    println!(
        "fixture:ok manifest_blake3={}",
        hex_string(receipt.manifest_hash())
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

fn observation(offset: u64, payload: Bytes) -> Result<SourceObservation, Box<dyn Error>> {
    Ok(SourceObservation::new(
        SourceId::new("primary-node")?,
        "node-v1.2.3",
        ObservationClass::CommittedBlock,
        SourceCursor::new("node-session-17", offset)?,
        ReceiveTimestamps::new(
            1_721_000_000_000_000 + i64::try_from(offset)?,
            99_000 + offset,
        )?,
        "parser-v1",
        payload,
        Vec::new(),
        1024 * 1024,
    )?)
}

fn hex_string(value: [u8; 32]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}
