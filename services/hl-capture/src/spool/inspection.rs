use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use storage_ports::{CursorPolicy, LocalRecordSequence};

use super::manifest::ClosedSegmentManifest;
use super::{
    RecoveryReport, SpoolError, SpoolReader, SpoolRecordSummary, io_error, recover_open_segment,
};

type SegmentPaths = BTreeMap<u64, PathBuf>;
type CollectedEntries = (SegmentPaths, SegmentPaths);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpoolInspectionBaseline {
    pub segment_sequence: u64,
    pub manifest_hash: [u8; 32],
    pub cursor_policy: CursorPolicy,
    pub local_sequence: Option<LocalRecordSequence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpoolInspection {
    closed_segments: u64,
    open_segments: u64,
    records: u64,
    chain_tip: Option<[u8; 32]>,
    segment_paths: Vec<PathBuf>,
    open_segment_path: Option<PathBuf>,
    last_sequence: Option<u64>,
}

impl SpoolInspection {
    #[must_use]
    pub const fn closed_segments(&self) -> u64 {
        self.closed_segments
    }

    #[must_use]
    pub const fn open_segments(&self) -> u64 {
        self.open_segments
    }

    #[must_use]
    pub const fn records(&self) -> u64 {
        self.records
    }

    #[must_use]
    pub const fn chain_tip(&self) -> Option<[u8; 32]> {
        self.chain_tip
    }

    #[must_use]
    pub fn segment_paths(&self) -> &[PathBuf] {
        &self.segment_paths
    }

    #[must_use]
    pub fn open_segment_path(&self) -> Option<&Path> {
        self.open_segment_path.as_deref()
    }

    #[must_use]
    pub const fn last_sequence(&self) -> Option<u64> {
        self.last_sequence
    }
}

pub fn inspect_spool(path: impl AsRef<Path>) -> Result<SpoolInspection, SpoolError> {
    inspect_spool_with_baseline(path, None)
}

pub(crate) fn inspect_spool_with_baseline(
    path: impl AsRef<Path>,
    baseline: Option<SpoolInspectionBaseline>,
) -> Result<SpoolInspection, SpoolError> {
    let path = path.as_ref();
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| io_error("reading spool inspection input", source))?;
    if metadata.file_type().is_symlink() {
        return Err(SpoolError::UnsafeSpoolEntry);
    }
    if metadata.is_file() {
        let reader = SpoolReader::open(path)?;
        let summary = reader.summarize()?;
        return Ok(SpoolInspection {
            closed_segments: 0,
            open_segments: 1,
            records: summary.record_count(),
            chain_tip: None,
            segment_paths: vec![path.to_owned()],
            open_segment_path: Some(path.to_owned()),
            last_sequence: Some(reader.header().segment_sequence()),
        });
    }
    if !metadata.is_dir() {
        return Err(SpoolError::UnsafeSpoolEntry);
    }
    inspect_directory(path, baseline)
}

pub fn recover_spool_tail(
    directory: impl AsRef<Path>,
) -> Result<Option<RecoveryReport>, SpoolError> {
    match inspect_spool(directory.as_ref()) {
        Ok(_) => return Ok(None),
        Err(SpoolError::IncompleteTail { .. }) => {}
        Err(error) => return Err(error),
    }
    let (segments, manifests) = collect_entries(directory.as_ref())?;
    let open = segments
        .iter()
        .filter(|(sequence, _)| !manifests.contains_key(sequence))
        .map(|(_, path)| path)
        .collect::<Vec<_>>();
    let [open] = open.as_slice() else {
        return Err(SpoolError::UnexpectedOpenSegment);
    };
    let report = recover_open_segment(open)?;
    inspect_spool(directory.as_ref())?;
    Ok(Some(report))
}

pub(crate) fn recover_spool_tail_with_baseline(
    directory: impl AsRef<Path>,
    baseline: Option<SpoolInspectionBaseline>,
) -> Result<Option<RecoveryReport>, SpoolError> {
    match inspect_spool_with_baseline(directory.as_ref(), baseline) {
        Ok(_) => return Ok(None),
        Err(SpoolError::IncompleteTail { .. }) => {}
        Err(error) => return Err(error),
    }
    let (segments, manifests) = collect_entries(directory.as_ref())?;
    let open = segments
        .iter()
        .filter(|(sequence, _)| !manifests.contains_key(sequence))
        .map(|(_, path)| path)
        .collect::<Vec<_>>();
    let [open] = open.as_slice() else {
        return Err(SpoolError::UnexpectedOpenSegment);
    };
    let report = recover_open_segment(open)?;
    inspect_spool_with_baseline(directory.as_ref(), baseline)?;
    Ok(Some(report))
}

fn inspect_directory(
    directory: &Path,
    baseline: Option<SpoolInspectionBaseline>,
) -> Result<SpoolInspection, SpoolError> {
    let (segments, manifests) = collect_entries(directory)?;
    inspect_directory_entries(segments, manifests, baseline)
}

fn collect_entries(directory: &Path) -> Result<CollectedEntries, SpoolError> {
    let mut segments = BTreeMap::new();
    let mut manifests = BTreeMap::new();
    for entry in
        fs::read_dir(directory).map_err(|source| io_error("reading the spool directory", source))?
    {
        let entry = entry.map_err(|source| io_error("reading a spool entry", source))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| SpoolError::UnsafeSpoolEntry)?;
        if name.starts_with("segment-") && name.ends_with(".hlsp.manifest.tmp") {
            return Err(SpoolError::IncompleteManifestPublication);
        }
        let Some((sequence, kind)) = parse_entry_name(&name) else {
            if name.starts_with("segment-") {
                return Err(SpoolError::UnsafeSpoolEntry);
            }
            continue;
        };
        if !entry
            .file_type()
            .map_err(|source| io_error("reading a spool entry type", source))?
            .is_file()
        {
            return Err(SpoolError::UnsafeSpoolEntry);
        }
        let collection = match kind {
            EntryKind::Segment => &mut segments,
            EntryKind::Manifest => &mut manifests,
        };
        if collection.insert(sequence, entry.path()).is_some() {
            return Err(SpoolError::DuplicateSegmentSequence);
        }
    }
    Ok((segments, manifests))
}

fn inspect_directory_entries(
    segments: SegmentPaths,
    manifests: SegmentPaths,
    baseline: Option<SpoolInspectionBaseline>,
) -> Result<SpoolInspection, SpoolError> {
    let mut closed_sequences = BTreeSet::new();
    let mut previous_sequence = baseline.map(|value| value.segment_sequence);
    let mut previous_manifest_hash = baseline.map(|value| value.manifest_hash);
    let mut previous_cursor_policy = baseline.map(|value| value.cursor_policy);
    let mut previous_local_sequence = baseline.and_then(|value| value.local_sequence);
    let mut total_records = 0_u64;
    for (sequence, manifest_path) in &manifests {
        if previous_sequence.is_some_and(|previous| previous.checked_add(1) != Some(*sequence)) {
            return Err(SpoolError::ManifestChainBroken);
        }
        let manifest = ClosedSegmentManifest::from_path(manifest_path)?;
        let expected_segment_name = format!("segment-{sequence:010}.hlsp");
        if manifest.segment_sequence() != *sequence
            || manifest.segment_file() != expected_segment_name
        {
            return Err(SpoolError::ManifestContentMismatch);
        }
        if manifest.previous_manifest_blake3() != previous_manifest_hash {
            return Err(SpoolError::ManifestChainBroken);
        }
        let segment_path = segments
            .get(sequence)
            .ok_or(SpoolError::ManifestSegmentMissing)?;
        verify_manifest_bytes(&manifest, segment_path)?;
        let reader = SpoolReader::open(segment_path)?;
        let summary = reader.summarize()?;
        verify_manifest_content(&manifest, &reader, &summary)?;
        let policy = reader.header().cursor_policy();
        if previous_cursor_policy.is_some_and(|previous| previous != policy) {
            return Err(SpoolError::ManifestChainBroken);
        }
        previous_cursor_policy = Some(policy);
        previous_local_sequence = verify_local_sequence_chain(
            &manifest,
            policy,
            summary.record_count(),
            previous_local_sequence,
        )?;

        let encoded = fs::read(manifest_path)
            .map_err(|source| io_error("hashing a closed-segment manifest", source))?;
        previous_manifest_hash = Some(*blake3::hash(&encoded).as_bytes());
        previous_sequence = Some(*sequence);
        closed_sequences.insert(*sequence);
        total_records = total_records
            .checked_add(summary.record_count())
            .ok_or(SpoolError::SizeOverflow)?;
    }

    let open_segments = segments
        .keys()
        .filter(|sequence| !closed_sequences.contains(sequence))
        .copied()
        .collect::<Vec<_>>();
    if open_segments.len() > 1 {
        return Err(SpoolError::UnexpectedOpenSegment);
    }
    if let (Some(open), Some(closed)) = (open_segments.first(), previous_sequence)
        && closed.checked_add(1) != Some(*open)
    {
        return Err(SpoolError::UnexpectedOpenSegment);
    }
    for sequence in &open_segments {
        let reader = SpoolReader::open(
            segments
                .get(sequence)
                .ok_or(SpoolError::UnexpectedOpenSegment)?,
        )?;
        let summary = reader.summarize()?;
        if previous_cursor_policy
            .is_some_and(|previous| previous != reader.header().cursor_policy())
        {
            return Err(SpoolError::ManifestChainBroken);
        }
        if reader.header().cursor_policy() == CursorPolicy::MonotonicByteOffset
            && summary.record_count() > 0
        {
            let first = match previous_local_sequence {
                Some(previous) => previous
                    .checked_next()
                    .map_err(|_| SpoolError::SizeOverflow)?,
                None => LocalRecordSequence::try_new(1).map_err(|_| SpoolError::SizeOverflow)?,
            };
            previous_local_sequence = Some(
                first
                    .checked_advance_by(summary.record_count() - 1)
                    .map_err(|_| SpoolError::SizeOverflow)?,
            );
        }
        total_records = total_records
            .checked_add(summary.record_count())
            .ok_or(SpoolError::SizeOverflow)?;
    }

    Ok(SpoolInspection {
        closed_segments: u64::try_from(manifests.len()).map_err(|_| SpoolError::SizeOverflow)?,
        open_segments: u64::try_from(open_segments.len()).map_err(|_| SpoolError::SizeOverflow)?,
        records: total_records,
        chain_tip: previous_manifest_hash,
        segment_paths: segments.values().cloned().collect(),
        open_segment_path: open_segments
            .first()
            .and_then(|sequence| segments.get(sequence))
            .cloned(),
        last_sequence: segments
            .keys()
            .next_back()
            .copied()
            .or_else(|| baseline.map(|value| value.segment_sequence)),
    })
}

pub(crate) fn verify_manifest_bytes(
    manifest: &ClosedSegmentManifest,
    segment_path: &Path,
) -> Result<(), SpoolError> {
    let metadata = fs::metadata(segment_path)
        .map_err(|source| io_error("reading closed segment metadata", source))?;
    if metadata.len() != manifest.file_size_bytes() {
        return Err(SpoolError::SegmentSizeMismatch);
    }
    if hash_file(segment_path)? != manifest.segment_blake3() {
        return Err(SpoolError::SegmentHashMismatch);
    }
    Ok(())
}

pub(crate) fn verify_manifest_content(
    manifest: &ClosedSegmentManifest,
    reader: &SpoolReader,
    summary: &SpoolRecordSummary,
) -> Result<(), SpoolError> {
    if reader.header().segment_sequence() != manifest.segment_sequence()
        || reader.header().source_id().as_str() != manifest.source_id()
        || reader.header().source_version() != manifest.source_version()
        || reader.header().schema_version() != manifest.spool_schema_version()
        || reader.header().producer_build_hash() != manifest.producer_build_hash()
        || manifest
            .cursor_policy()
            .is_some_and(|policy| reader.header().cursor_policy() != policy)
    {
        return Err(SpoolError::ManifestContentMismatch);
    }
    if manifest.record_count() != summary.record_count()
        || Some(manifest.min_cursor()) != summary.first_cursor()
        || Some(manifest.max_cursor()) != summary.last_cursor()
    {
        return Err(SpoolError::ManifestContentMismatch);
    }
    match (
        manifest.first_local_sequence(),
        manifest.last_local_sequence(),
    ) {
        (None, None) => {}
        (Some(first), Some(last)) => {
            let expected_last = first
                .checked_advance_by(
                    summary
                        .record_count()
                        .checked_sub(1)
                        .ok_or(SpoolError::ManifestContentMismatch)?,
                )
                .map_err(|_| SpoolError::ManifestContentMismatch)?;
            if expected_last != last {
                return Err(SpoolError::ManifestContentMismatch);
            }
        }
        _ => return Err(SpoolError::ManifestContentMismatch),
    }
    Ok(())
}

fn verify_local_sequence_chain(
    manifest: &ClosedSegmentManifest,
    policy: CursorPolicy,
    record_count: u64,
    previous: Option<LocalRecordSequence>,
) -> Result<Option<LocalRecordSequence>, SpoolError> {
    match policy {
        CursorPolicy::ContiguousNativeOffset => {
            if manifest.first_local_sequence().is_some() || manifest.last_local_sequence().is_some()
            {
                return Err(SpoolError::ManifestContentMismatch);
            }
            Ok(None)
        }
        CursorPolicy::MonotonicByteOffset => {
            let expected_first = match previous {
                Some(previous) => previous
                    .checked_next()
                    .map_err(|_| SpoolError::SizeOverflow)?,
                None => LocalRecordSequence::try_new(1).map_err(|_| SpoolError::SizeOverflow)?,
            };
            if let Some(declared_first) = manifest.first_local_sequence()
                && declared_first != expected_first
            {
                return Err(SpoolError::ManifestChainBroken);
            }
            let expected_last = expected_first
                .checked_advance_by(
                    record_count
                        .checked_sub(1)
                        .ok_or(SpoolError::ManifestContentMismatch)?,
                )
                .map_err(|_| SpoolError::SizeOverflow)?;
            if let Some(declared_last) = manifest.last_local_sequence()
                && declared_last != expected_last
            {
                return Err(SpoolError::ManifestChainBroken);
            }
            Ok(Some(expected_last))
        }
    }
}

fn hash_file(path: &Path) -> Result<[u8; 32], SpoolError> {
    let mut file =
        File::open(path).map_err(|source| io_error("opening a segment for hashing", source))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| io_error("hashing a spool segment", source))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(*hasher.finalize().as_bytes())
}

#[derive(Debug, Clone, Copy)]
enum EntryKind {
    Segment,
    Manifest,
}

fn parse_entry_name(name: &str) -> Option<(u64, EntryKind)> {
    let (sequence, kind) = if let Some(sequence) = name
        .strip_prefix("segment-")
        .and_then(|value| value.strip_suffix(".hlsp"))
    {
        (sequence, EntryKind::Segment)
    } else {
        let sequence = name
            .strip_prefix("segment-")
            .and_then(|value| value.strip_suffix(".hlsp.manifest"))?;
        (sequence, EntryKind::Manifest)
    };
    if sequence.len() < 10 || !sequence.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    sequence.parse().ok().map(|sequence| (sequence, kind))
}
