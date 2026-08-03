use std::fs::OpenOptions;
use std::path::Path;

use super::header::SegmentHeader;
use super::manifest::manifest_path_for;
use super::reader::scan_records;
use super::{SpoolError, io_error};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryReport {
    pub valid_records: u64,
    pub truncated_bytes: u64,
    pub final_size: u64,
}

pub fn recover_open_segment(path: impl AsRef<Path>) -> Result<RecoveryReport, SpoolError> {
    match manifest_path_for(path.as_ref()).symlink_metadata() {
        Ok(_) => return Err(SpoolError::ClosedSegment),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(io_error("checking segment close state", source)),
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.as_ref())
        .map_err(|source| io_error("opening an open segment for recovery", source))?;
    let (_, records_offset) = SegmentHeader::read_from(&mut file)?;
    let initial_size = file
        .metadata()
        .map_err(|source| io_error("reading segment metadata before recovery", source))?
        .len();
    let scan = scan_records(&mut file, records_offset)?;
    let final_size = if scan.incomplete_tail.is_some() {
        file.set_len(scan.last_valid_offset)
            .map_err(|source| io_error("truncating an incomplete spool tail", source))?;
        file.sync_all()
            .map_err(|source| io_error("syncing a recovered spool segment", source))?;
        scan.last_valid_offset
    } else {
        initial_size
    };
    Ok(RecoveryReport {
        valid_records: scan.record_count,
        truncated_bytes: initial_size
            .checked_sub(final_size)
            .ok_or(SpoolError::SizeOverflow)?,
        final_size,
    })
}
