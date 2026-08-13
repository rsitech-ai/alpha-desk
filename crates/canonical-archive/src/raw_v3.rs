use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use domain_types::{ChainId, KnownTime, SourceId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use storage_ports::{
    ArchiveError, RAW_ARCHIVE_MAXIMUM_DATA_PACK_BYTES,
    RAW_ARCHIVE_MAXIMUM_EMBEDDED_PACK_MANIFEST_BYTES, RAW_ARCHIVE_MAXIMUM_INDEX_PACK_BYTES,
    RAW_ARCHIVE_MAXIMUM_LOGICAL_MANIFEST_BYTES, RAW_ARCHIVE_MAXIMUM_PACK_LOGICAL_INPUTS,
    RAW_ARCHIVE_MAXIMUM_RELATIVE_PATH_BYTES, RAW_ARCHIVE_MAXIMUM_SEQUENCE_PAGE_BYTES,
    RAW_ARCHIVE_MAXIMUM_SEQUENCE_TREE_DEPTH,
};

use super::{fs, manifest, schema};

pub const RAW_SEQUENCE_LEAF_SCHEMA_V3: &str = "hyperliquid-alpha-desk/archive-raw-sequence-leaf/v3";
pub const RAW_SEQUENCE_INTERNAL_SCHEMA_V3: &str =
    "hyperliquid-alpha-desk/archive-raw-sequence-internal/v3";
pub const RAW_ROOT_BUNDLE_SCHEMA_V3: &str = "hyperliquid-alpha-desk/archive-raw-root-bundle/v3";
pub const RAW_BYTE_DATASET_V3: &str = "raw_source_observations_byte_v3";
pub const RAW_RECEIPT_HINT_PAGE_SCHEMA_V3: &str =
    "hyperliquid-alpha-desk/archive-raw-receipt-hint-page/v3";
pub const RAW_INDEX_PACK_MANIFEST_SCHEMA_V3: &str =
    "hyperliquid-alpha-desk/archive-raw-index-pack-manifest/v3";
pub const RAW_PACK_MANIFEST_SCHEMA_V3: &str = "hyperliquid-alpha-desk/archive-raw-pack-manifest/v3";
const SEQUENCE_PAGE_HASH_DOMAIN_V3: &[u8] =
    b"hyperliquid-alpha-desk:archive-raw-sequence-leaf:v3\0";
const SEQUENCE_INTERNAL_HASH_DOMAIN_V3: &[u8] =
    b"hyperliquid-alpha-desk:archive-raw-sequence-internal:v3\0";
const RECEIPT_HINT_HASH_DOMAIN_V3: &[u8] = b"hyperliquid-alpha-desk:archive-raw-receipt-hint:v3\0";
const ROOT_BUNDLE_HASH_DOMAIN_V3: &[u8] = b"hyperliquid-alpha-desk:archive-raw-root-bundle:v3\0";
const PACK_COMBINED_HASH_DOMAIN_V3: &[u8] =
    b"hyperliquid-alpha-desk:archive-raw-pack-combined:v3\0";
const JOURNAL_PREFIX_HASH_DOMAIN_V3: &[u8] =
    b"hyperliquid-alpha-desk:archive-raw-journal-prefix:v3\0";
const JOURNAL_FRAME_MAGIC_V3: &[u8; 8] = b"HADJRV3\0";
const JOURNAL_FRAME_HEADER_BYTES: u64 = 64;
pub(crate) const MAX_SEQUENCE_LEAF_ENTRIES: usize = 256;
const MAX_SEQUENCE_INTERNAL_CHILDREN: usize = 256;
const MAX_RECEIPT_HINT_ENTRIES: usize = 256;
const MAX_INDEX_PACK_PAGES: usize = 4_096;
pub(crate) const MAX_JOURNAL_RECORDS: u64 = 4_096;
pub(crate) const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TEXT_FIELD_BYTES: usize = 256;
pub const RAW_LOGICAL_COMMIT_SCHEMA_V3: &str =
    "hyperliquid-alpha-desk/archive-raw-logical-commit/v3";
const LOGICAL_COMMIT_HASH_DOMAIN_V3: &[u8] =
    b"hyperliquid-alpha-desk:archive-raw-logical-commit:v3\0";
const RAW_V3_CURSOR_POLICY: &str = "monotonic-byte-offset";
pub(crate) const RAW_V3_ROLLING_HASH_DOMAIN: &[u8] =
    b"hyperliquid-alpha-desk/raw-rolling-content/v3";
const RAW_V3_JOURNAL_FILE_IDENTITY_PREFIX: &str = "generation-";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JournalPrefixRefV3 {
    generation: u64,
    file_identity: String,
    relative_path: String,
    committed_prefix_length: u64,
    committed_record_count: u64,
    committed_prefix_sha256: String,
    root_record_sequence: u64,
    root_first_local_sequence: u64,
    root_last_local_sequence: u64,
    root_row_count: u64,
    root_logical_manifest_count: u64,
    root_page_domain_sha256: String,
}

impl JournalPrefixRefV3 {
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn file_identity(&self) -> &str {
        &self.file_identity
    }

    #[must_use]
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    #[must_use]
    pub const fn committed_prefix_length(&self) -> u64 {
        self.committed_prefix_length
    }

    #[must_use]
    pub const fn committed_record_count(&self) -> u64 {
        self.committed_record_count
    }

    pub fn committed_prefix_sha256(&self) -> Result<[u8; 32], ArchiveError> {
        manifest::parse_hash(&self.committed_prefix_sha256)
    }

    #[must_use]
    pub const fn root_record_sequence(&self) -> u64 {
        self.root_record_sequence
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum SequenceStorageRefV3 {
    Logical {
        manifest_relative_path: String,
        manifest_sha256: String,
    },
    Packed {
        pack_manifest_relative_path: String,
        pack_manifest_sha256: String,
    },
}

impl SequenceStorageRefV3 {
    pub fn logical_manifest_sha256(&self) -> Result<Option<[u8; 32]>, ArchiveError> {
        match self {
            Self::Logical {
                manifest_sha256, ..
            } => manifest::parse_hash(manifest_sha256).map(Some),
            Self::Packed { .. } => Ok(None),
        }
    }

    #[must_use]
    pub fn logical_manifest_relative_path(&self) -> Option<&str> {
        match self {
            Self::Logical {
                manifest_relative_path,
                ..
            } => Some(manifest_relative_path),
            Self::Packed { .. } => None,
        }
    }

    pub fn pack_manifest_sha256(&self) -> Result<Option<[u8; 32]>, ArchiveError> {
        match self {
            Self::Packed {
                pack_manifest_sha256,
                ..
            } => manifest::parse_hash(pack_manifest_sha256).map(Some),
            Self::Logical { .. } => Ok(None),
        }
    }

    #[must_use]
    pub fn pack_manifest_relative_path(&self) -> Option<&str> {
        match self {
            Self::Packed {
                pack_manifest_relative_path,
                ..
            } => Some(pack_manifest_relative_path),
            Self::Logical { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceLeafEntryV3 {
    first_local_sequence: u64,
    last_local_sequence: u64,
    partition: String,
    object_size_bytes: u64,
    row_count: u64,
    logical_manifest_count: u64,
    storage: SequenceStorageRefV3,
}

impl SequenceLeafEntryV3 {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new_logical(
        first_local_sequence: u64,
        last_local_sequence: u64,
        manifest_relative_path: impl Into<String>,
        manifest_sha256: [u8; 32],
        object_size_bytes: u64,
        row_count: u64,
        partition: impl Into<String>,
    ) -> Result<Self, ArchiveError> {
        let expected_rows = sequence_span(first_local_sequence, last_local_sequence)?;
        if object_size_bytes == 0 || row_count != expected_rows {
            return Err(ArchiveError::InvalidInput(
                "logical sequence entry size or row coverage",
            ));
        }
        let manifest_relative_path = manifest_relative_path.into();
        let manifest_relative_path = checked_relative_string(Path::new(&manifest_relative_path))?;
        let partition = partition.into();
        validate_partition(&partition)?;
        Ok(Self {
            first_local_sequence,
            last_local_sequence,
            partition,
            object_size_bytes,
            row_count,
            logical_manifest_count: 1,
            storage: SequenceStorageRefV3::Logical {
                manifest_relative_path,
                manifest_sha256: hex::encode(manifest_sha256),
            },
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn try_new_packed(
        first_local_sequence: u64,
        last_local_sequence: u64,
        pack_manifest_relative_path: impl Into<String>,
        pack_manifest_sha256: [u8; 32],
        object_size_bytes: u64,
        row_count: u64,
        logical_manifest_count: u64,
        partition: impl Into<String>,
    ) -> Result<Self, ArchiveError> {
        let expected_rows = sequence_span(first_local_sequence, last_local_sequence)?;
        if object_size_bytes == 0
            || row_count != expected_rows
            || logical_manifest_count < 2
            || logical_manifest_count > row_count
        {
            return Err(ArchiveError::InvalidInput(
                "packed sequence entry size, row coverage, or logical count",
            ));
        }
        let pack_manifest_relative_path = pack_manifest_relative_path.into();
        let pack_manifest_relative_path =
            checked_relative_string(Path::new(&pack_manifest_relative_path))?;
        let partition = partition.into();
        validate_partition(&partition)?;
        Ok(Self {
            first_local_sequence,
            last_local_sequence,
            partition,
            object_size_bytes,
            row_count,
            logical_manifest_count,
            storage: SequenceStorageRefV3::Packed {
                pack_manifest_relative_path,
                pack_manifest_sha256: hex::encode(pack_manifest_sha256),
            },
        })
    }

    #[must_use]
    pub const fn logical_manifest_count(&self) -> u64 {
        self.logical_manifest_count
    }

    #[must_use]
    pub const fn first_local_sequence(&self) -> u64 {
        self.first_local_sequence
    }

    #[must_use]
    pub const fn last_local_sequence(&self) -> u64 {
        self.last_local_sequence
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }

    #[must_use]
    pub const fn object_size_bytes(&self) -> u64 {
        self.object_size_bytes
    }

    #[must_use]
    pub fn partition(&self) -> &str {
        &self.partition
    }

    #[must_use]
    pub const fn storage(&self) -> &SequenceStorageRefV3 {
        &self.storage
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptHintEntryV3 {
    manifest_sha256: String,
    first_local_sequence: u64,
    last_local_sequence: u64,
}

impl ReceiptHintEntryV3 {
    pub fn try_new(
        manifest_sha256: [u8; 32],
        first_local_sequence: u64,
        last_local_sequence: u64,
    ) -> Result<Self, ArchiveError> {
        sequence_span(first_local_sequence, last_local_sequence)?;
        Ok(Self {
            manifest_sha256: hex::encode(manifest_sha256),
            first_local_sequence,
            last_local_sequence,
        })
    }

    pub fn manifest_sha256(&self) -> Result<[u8; 32], ArchiveError> {
        manifest::parse_hash(&self.manifest_sha256)
    }

    #[must_use]
    pub const fn first_local_sequence(&self) -> u64 {
        self.first_local_sequence
    }

    #[must_use]
    pub const fn last_local_sequence(&self) -> u64 {
        self.last_local_sequence
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptHintPageV3 {
    schema: String,
    chain_id: String,
    source_id: String,
    dataset: String,
    authoritative: bool,
    entries: Vec<ReceiptHintEntryV3>,
}

impl ReceiptHintPageV3 {
    pub fn try_new(
        chain_id: ChainId,
        source_id: SourceId,
        entries: Vec<ReceiptHintEntryV3>,
    ) -> Result<Self, ArchiveError> {
        if entries.is_empty() || entries.len() > MAX_RECEIPT_HINT_ENTRIES {
            return Err(ArchiveError::InvalidInput(
                "receipt hint page fanout must be bounded and nonzero",
            ));
        }
        if entries
            .windows(2)
            .any(|pair| pair[0].manifest_sha256 >= pair[1].manifest_sha256)
        {
            return Err(ArchiveError::InvalidInput(
                "receipt hint keys must be strictly sorted and unique",
            ));
        }
        Ok(Self {
            schema: RAW_RECEIPT_HINT_PAGE_SCHEMA_V3.to_owned(),
            chain_id: chain_id.as_str().to_owned(),
            source_id: source_id.as_str().to_owned(),
            dataset: RAW_BYTE_DATASET_V3.to_owned(),
            authoritative: false,
            entries,
        })
    }

    #[must_use]
    pub fn candidate_range(&self, manifest_sha256: [u8; 32]) -> Option<(u64, u64)> {
        let key = hex::encode(manifest_sha256);
        self.entries
            .binary_search_by(|entry| entry.manifest_sha256.cmp(&key))
            .ok()
            .map(|index| {
                let entry = &self.entries[index];
                (entry.first_local_sequence, entry.last_local_sequence)
            })
    }

    #[must_use]
    pub fn entries(&self) -> &[ReceiptHintEntryV3] {
        &self.entries
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum IndexPackPageKindV3 {
    SequenceLeaf,
    SequenceInternal,
    ReceiptHint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IndexPackPageRefV3 {
    kind: IndexPackPageKindV3,
    offset: u64,
    length: u64,
    page_domain_sha256: String,
}

impl IndexPackPageRefV3 {
    fn try_new(
        kind: IndexPackPageKindV3,
        offset: u64,
        length: u64,
        page_domain_sha256: [u8; 32],
    ) -> Result<Self, ArchiveError> {
        if length == 0 || offset.checked_add(length).is_none() {
            return Err(ArchiveError::InvalidInput("index pack page slice"));
        }
        Ok(Self {
            kind,
            offset,
            length,
            page_domain_sha256: hex::encode(page_domain_sha256),
        })
    }

    fn end(&self) -> Result<u64, ArchiveError> {
        self.offset
            .checked_add(self.length)
            .ok_or(ArchiveError::InvalidInput(
                "index pack page slice overflows",
            ))
    }

    #[must_use]
    pub const fn kind(&self) -> IndexPackPageKindV3 {
        self.kind
    }

    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    #[must_use]
    pub const fn length(&self) -> u64 {
        self.length
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IndexPackManifestV3 {
    schema: String,
    chain_id: String,
    source_id: String,
    dataset: String,
    generation: u64,
    object_relative_path: String,
    object_sha256: String,
    object_size_bytes: u64,
    pages: Vec<IndexPackPageRefV3>,
}

impl IndexPackManifestV3 {
    fn from_exact_bytes(
        chain_id: ChainId,
        source_id: SourceId,
        generation: u64,
        object_bytes: &[u8],
        pages: Vec<IndexPackPageRefV3>,
    ) -> Result<Self, ArchiveError> {
        let object_size_bytes = u64::try_from(object_bytes.len())
            .map_err(|_| ArchiveError::InvalidInput("index pack exceeds u64"))?;
        if generation == 0
            || object_size_bytes == 0
            || object_size_bytes > RAW_ARCHIVE_MAXIMUM_INDEX_PACK_BYTES
            || pages.is_empty()
            || pages.len() > MAX_INDEX_PACK_PAGES
        {
            return Err(ArchiveError::InvalidInput(
                "index pack generation, size, or page count",
            ));
        }
        let mut previous_end = 0_u64;
        for page in &pages {
            let end = page.end()?;
            if page.offset != previous_end || end > object_size_bytes {
                return Err(ArchiveError::InvalidInput(
                    "index pack page slices are not exact and contiguous",
                ));
            }
            let start = usize::try_from(page.offset)
                .map_err(|_| ArchiveError::InvalidInput("index pack page offset exceeds usize"))?;
            let end_usize = usize::try_from(end)
                .map_err(|_| ArchiveError::InvalidInput("index pack page end exceeds usize"))?;
            if page_domain_hash(page.kind, &object_bytes[start..end_usize])?
                != manifest::parse_hash(&page.page_domain_sha256)?
            {
                return Err(ArchiveError::ManifestVerification(
                    "index pack page hash does not authenticate its exact bytes",
                ));
            }
            previous_end = end;
        }
        if previous_end != object_size_bytes {
            return Err(ArchiveError::InvalidInput(
                "index pack pages do not cover the exact object",
            ));
        }
        let object_sha256 = manifest::sha256(object_bytes);
        let object_relative_path = format!("index-packs/{}.pack", hex::encode(object_sha256));
        Ok(Self {
            schema: RAW_INDEX_PACK_MANIFEST_SCHEMA_V3.to_owned(),
            chain_id: chain_id.as_str().to_owned(),
            source_id: source_id.as_str().to_owned(),
            dataset: RAW_BYTE_DATASET_V3.to_owned(),
            generation,
            object_relative_path,
            object_sha256: hex::encode(object_sha256),
            object_size_bytes,
            pages,
        })
    }

    #[must_use]
    pub fn pages(&self) -> &[IndexPackPageRefV3] {
        &self.pages
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    #[must_use]
    pub fn object_relative_path(&self) -> &str {
        &self.object_relative_path
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn object_sha256(&self) -> Result<[u8; 32], ArchiveError> {
        manifest::parse_hash(&self.object_sha256)
    }

    fn verify_bytes(&self, object_bytes: &[u8]) -> Result<(), ArchiveError> {
        let expected_size = u64::try_from(object_bytes.len())
            .map_err(|_| ArchiveError::InvalidInput("index pack exceeds u64"))?;
        if expected_size != self.object_size_bytes
            || manifest::sha256(object_bytes) != manifest::parse_hash(&self.object_sha256)?
        {
            return Err(ArchiveError::ManifestVerification(
                "index pack object bytes or hash are invalid",
            ));
        }
        let rebuilt = Self::from_exact_bytes(
            ChainId::new(self.chain_id.clone())
                .map_err(|_| ArchiveError::ManifestVerification("invalid index pack chain"))?,
            SourceId::new(self.source_id.clone())
                .map_err(|_| ArchiveError::ManifestVerification("invalid index pack source"))?,
            self.generation,
            object_bytes,
            self.pages.clone(),
        )?;
        if &rebuilt != self {
            return Err(ArchiveError::ManifestVerification(
                "index pack manifest is not derived from its exact object",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct IndexPackBuilderV3 {
    chain_id: ChainId,
    source_id: SourceId,
    generation: u64,
    bytes: Vec<u8>,
    pages: Vec<IndexPackPageRefV3>,
}

impl IndexPackBuilderV3 {
    pub fn try_new(
        chain_id: ChainId,
        source_id: SourceId,
        generation: u64,
    ) -> Result<Self, ArchiveError> {
        if generation == 0 {
            return Err(ArchiveError::InvalidInput(
                "index pack generation must be nonzero",
            ));
        }
        Ok(Self {
            chain_id,
            source_id,
            generation,
            bytes: Vec::new(),
            pages: Vec::new(),
        })
    }

    pub fn push_sequence_leaf(&mut self, page: &SequenceLeafPageV3) -> Result<usize, ArchiveError> {
        self.validate_page_identity(&page.chain_id, &page.source_id)?;
        self.push_page(
            IndexPackPageKindV3::SequenceLeaf,
            manifest::canonical_json(page)?,
        )
    }

    pub fn push_sequence_internal(
        &mut self,
        page: &SequenceInternalPageV3,
    ) -> Result<usize, ArchiveError> {
        self.validate_page_identity(&page.chain_id, &page.source_id)?;
        self.push_page(
            IndexPackPageKindV3::SequenceInternal,
            manifest::canonical_json(page)?,
        )
    }

    pub fn push_receipt_hint(&mut self, page: &ReceiptHintPageV3) -> Result<usize, ArchiveError> {
        self.validate_page_identity(&page.chain_id, &page.source_id)?;
        self.push_page(
            IndexPackPageKindV3::ReceiptHint,
            manifest::canonical_json(page)?,
        )
    }

    pub fn finish(self) -> Result<BuiltIndexPackV3, ArchiveError> {
        let manifest = IndexPackManifestV3::from_exact_bytes(
            self.chain_id,
            self.source_id,
            self.generation,
            &self.bytes,
            self.pages,
        )?;
        Ok(BuiltIndexPackV3 {
            bytes: self.bytes,
            manifest,
        })
    }

    fn validate_page_identity(&self, chain_id: &str, source_id: &str) -> Result<(), ArchiveError> {
        if chain_id != self.chain_id.as_str() || source_id != self.source_id.as_str() {
            return Err(ArchiveError::InvalidInput(
                "index pack page chain or source mismatch",
            ));
        }
        Ok(())
    }

    fn push_page(
        &mut self,
        kind: IndexPackPageKindV3,
        page_bytes: Vec<u8>,
    ) -> Result<usize, ArchiveError> {
        let new_object_size =
            self.bytes
                .len()
                .checked_add(page_bytes.len())
                .ok_or(ArchiveError::InvalidInput(
                    "index pack object size overflows",
                ))?;
        let new_object_size = u64::try_from(new_object_size)
            .map_err(|_| ArchiveError::InvalidInput("index pack object exceeds u64"))?;
        let page_size = u64::try_from(page_bytes.len())
            .map_err(|_| ArchiveError::InvalidInput("index pack page exceeds u64"))?;
        if page_bytes.is_empty()
            || page_size > RAW_ARCHIVE_MAXIMUM_SEQUENCE_PAGE_BYTES
            || self.pages.len() >= MAX_INDEX_PACK_PAGES
            || new_object_size > RAW_ARCHIVE_MAXIMUM_INDEX_PACK_BYTES
        {
            return Err(ArchiveError::InvalidInput(
                "index pack page or object bound",
            ));
        }
        let offset = u64::try_from(self.bytes.len())
            .map_err(|_| ArchiveError::InvalidInput("index pack offset exceeds u64"))?;
        let length = page_size;
        let page_hash = page_domain_hash(kind, &page_bytes)?;
        let page_index = self.pages.len();
        self.bytes.extend_from_slice(&page_bytes);
        self.pages.push(IndexPackPageRefV3::try_new(
            kind, offset, length, page_hash,
        )?);
        Ok(page_index)
    }
}

#[derive(Debug, Clone)]
pub struct BuiltIndexPackV3 {
    bytes: Vec<u8>,
    manifest: IndexPackManifestV3,
}

impl BuiltIndexPackV3 {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub const fn manifest(&self) -> &IndexPackManifestV3 {
        &self.manifest
    }

    pub fn object_sha256(&self) -> [u8; 32] {
        manifest::sha256(&self.bytes)
    }

    pub fn verify_bytes(&self, object_bytes: &[u8]) -> Result<(), ArchiveError> {
        self.manifest.verify_bytes(object_bytes)
    }

    pub fn sequence_leaf_ref(
        &self,
        page_index: usize,
        page: &SequenceLeafPageV3,
    ) -> Result<SequenceNodeRefV3, ArchiveError> {
        let locator = self.sequence_locator(
            page_index,
            IndexPackPageKindV3::SequenceLeaf,
            &manifest::canonical_json(page)?,
        )?;
        Ok(SequenceNodeRefV3::from_leaf(page, locator))
    }

    pub fn sequence_internal_ref(
        &self,
        page_index: usize,
        page: &SequenceInternalPageV3,
    ) -> Result<SequenceNodeRefV3, ArchiveError> {
        let locator = self.sequence_locator(
            page_index,
            IndexPackPageKindV3::SequenceInternal,
            &manifest::canonical_json(page)?,
        )?;
        Ok(SequenceNodeRefV3::from_internal(page, locator))
    }

    fn sequence_locator(
        &self,
        page_index: usize,
        expected_kind: IndexPackPageKindV3,
        expected_bytes: &[u8],
    ) -> Result<SequencePageLocatorV3, ArchiveError> {
        let page = self
            .manifest
            .pages
            .get(page_index)
            .ok_or(ArchiveError::InvalidInput("index pack page index"))?;
        let end = usize::try_from(page.end()?)
            .map_err(|_| ArchiveError::InvalidInput("index pack page end exceeds usize"))?;
        let start = usize::try_from(page.offset)
            .map_err(|_| ArchiveError::InvalidInput("index pack page offset exceeds usize"))?;
        if page.kind != expected_kind || self.bytes.get(start..end) != Some(expected_bytes) {
            return Err(ArchiveError::ManifestVerification(
                "sequence page does not match the built index pack",
            ));
        }
        SequencePageLocatorV3::index_pack(
            manifest::parse_hash(&self.manifest.object_sha256)?,
            page.offset,
            page.length,
            manifest::parse_hash(&page.page_domain_sha256)?,
        )
    }
}

pub(crate) type IndexPackBytes = BTreeMap<[u8; 32], Vec<u8>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogicalObjectDescriptorV3 {
    relative_path: String,
    sha256: String,
    size_bytes: u64,
    row_count: u64,
    schema_fingerprint_sha256: String,
}

impl LogicalObjectDescriptorV3 {
    pub fn try_new(
        relative_path: PathBuf,
        sha256: [u8; 32],
        size_bytes: u64,
        row_count: u64,
    ) -> Result<Self, ArchiveError> {
        if size_bytes == 0 || row_count == 0 {
            return Err(ArchiveError::InvalidInput(
                "logical object size and row count must be nonzero",
            ));
        }
        Ok(Self {
            relative_path: checked_relative_string(&relative_path)?,
            sha256: hex::encode(sha256),
            size_bytes,
            row_count,
            schema_fingerprint_sha256: hex::encode(schema::raw_schema_fingerprint()?),
        })
    }

    #[must_use]
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    pub fn sha256(&self) -> Result<[u8; 32], ArchiveError> {
        manifest::parse_hash(&self.sha256)
    }

    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogicalCommitDescriptorV3 {
    chain_id: String,
    source_id: String,
    source_version: String,
    observation_class: String,
    cursor_policy: String,
    cursor_epoch: String,
    start_offset: u64,
    end_offset: u64,
    first_local_sequence: u64,
    last_local_sequence: u64,
    first_received_wall_micros: i64,
    last_received_wall_micros: i64,
    parser_schema_version: String,
    spool_manifest_blake3: String,
    spool_segment_blake3: String,
    rolling_content_sha256: String,
}

impl LogicalCommitDescriptorV3 {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        chain_id: ChainId,
        source_id: SourceId,
        source_version: impl Into<String>,
        observation_class: impl Into<String>,
        cursor_epoch: impl Into<String>,
        start_offset: u64,
        end_offset: u64,
        first_local_sequence: u64,
        last_local_sequence: u64,
        first_received_wall_micros: i64,
        last_received_wall_micros: i64,
        parser_schema_version: impl Into<String>,
        spool_manifest_blake3: [u8; 32],
        spool_segment_blake3: [u8; 32],
        rolling_content_sha256: [u8; 32],
    ) -> Result<Self, ArchiveError> {
        let source_version = source_version.into();
        let observation_class = observation_class.into();
        let cursor_epoch = cursor_epoch.into();
        let parser_schema_version = parser_schema_version.into();
        validate_text(&source_version, "logical commit source version")?;
        validate_text(&observation_class, "logical commit observation class")?;
        validate_text(&cursor_epoch, "logical commit cursor epoch")?;
        validate_text(&parser_schema_version, "logical commit parser schema")?;
        super::raw::parse_observation_class(&observation_class)?;
        let row_count = sequence_span(first_local_sequence, last_local_sequence)?;
        if start_offset > end_offset
            || first_received_wall_micros < 0
            || last_received_wall_micros < 0
            || first_received_wall_micros > last_received_wall_micros
            || row_count == 0
        {
            return Err(ArchiveError::InvalidInput(
                "logical commit cursor, time, or sequence range",
            ));
        }
        let first_partition = manifest::partition_for(first_received_wall_micros)?;
        let last_partition = manifest::partition_for(last_received_wall_micros)?;
        if first_partition != last_partition {
            return Err(ArchiveError::InvalidInput(
                "logical commit crosses an hour partition",
            ));
        }
        Ok(Self {
            chain_id: chain_id.as_str().to_owned(),
            source_id: source_id.as_str().to_owned(),
            source_version,
            observation_class,
            cursor_policy: RAW_V3_CURSOR_POLICY.to_owned(),
            cursor_epoch,
            start_offset,
            end_offset,
            first_local_sequence,
            last_local_sequence,
            first_received_wall_micros,
            last_received_wall_micros,
            parser_schema_version,
            spool_manifest_blake3: hex::encode(spool_manifest_blake3),
            spool_segment_blake3: hex::encode(spool_segment_blake3),
            rolling_content_sha256: hex::encode(rolling_content_sha256),
        })
    }

    pub fn chain_id(&self) -> Result<ChainId, ArchiveError> {
        ChainId::new(self.chain_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("invalid logical commit chain"))
    }

    pub fn source_id(&self) -> Result<SourceId, ArchiveError> {
        SourceId::new(self.source_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("invalid logical commit source"))
    }

    #[must_use]
    pub const fn first_local_sequence(&self) -> u64 {
        self.first_local_sequence
    }

    #[must_use]
    pub const fn last_local_sequence(&self) -> u64 {
        self.last_local_sequence
    }

    #[must_use]
    pub fn cursor_epoch(&self) -> &str {
        &self.cursor_epoch
    }

    #[must_use]
    pub const fn start_offset(&self) -> u64 {
        self.start_offset
    }

    #[must_use]
    pub const fn end_offset(&self) -> u64 {
        self.end_offset
    }

    pub fn rolling_content_sha256(&self) -> Result<[u8; 32], ArchiveError> {
        manifest::parse_hash(&self.rolling_content_sha256)
    }

    pub fn spool_manifest_blake3(&self) -> Result<[u8; 32], ArchiveError> {
        manifest::parse_hash(&self.spool_manifest_blake3)
    }

    pub fn spool_segment_blake3(&self) -> Result<[u8; 32], ArchiveError> {
        manifest::parse_hash(&self.spool_segment_blake3)
    }

    pub fn partition(&self) -> Result<String, ArchiveError> {
        manifest::partition_for(self.first_received_wall_micros)
    }

    #[must_use]
    pub fn source_version(&self) -> &str {
        &self.source_version
    }

    #[must_use]
    pub fn observation_class(&self) -> &str {
        &self.observation_class
    }

    #[must_use]
    pub fn parser_schema_version(&self) -> &str {
        &self.parser_schema_version
    }

    #[must_use]
    pub const fn first_received_wall_micros(&self) -> i64 {
        self.first_received_wall_micros
    }

    #[must_use]
    pub const fn last_received_wall_micros(&self) -> i64 {
        self.last_received_wall_micros
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogicalCommitManifestV3 {
    schema: String,
    producer_build_id: String,
    created_at_micros: i64,
    commit: LogicalCommitDescriptorV3,
    object: LogicalObjectDescriptorV3,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogicalCommitManifestWireV3 {
    schema: String,
    producer_build_id: String,
    created_at_micros: i64,
    commit: LogicalCommitDescriptorWireV3,
    object: LogicalObjectDescriptorWireV3,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogicalCommitDescriptorWireV3 {
    chain_id: String,
    source_id: String,
    source_version: String,
    observation_class: String,
    cursor_policy: String,
    cursor_epoch: String,
    start_offset: u64,
    end_offset: u64,
    first_local_sequence: u64,
    last_local_sequence: u64,
    first_received_wall_micros: i64,
    last_received_wall_micros: i64,
    parser_schema_version: String,
    spool_manifest_blake3: String,
    spool_segment_blake3: String,
    rolling_content_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogicalObjectDescriptorWireV3 {
    relative_path: String,
    sha256: String,
    size_bytes: u64,
    row_count: u64,
    schema_fingerprint_sha256: String,
}

impl LogicalCommitManifestV3 {
    pub fn try_new(
        producer_build_id: impl Into<String>,
        created_at: KnownTime,
        commit: LogicalCommitDescriptorV3,
        object: LogicalObjectDescriptorV3,
    ) -> Result<Self, ArchiveError> {
        let producer_build_id = producer_build_id.into();
        validate_text(&producer_build_id, "logical commit producer build ID")?;
        let expected_rows = sequence_span(commit.first_local_sequence, commit.last_local_sequence)?;
        if object.row_count != expected_rows
            || object.schema_fingerprint_sha256 != hex::encode(schema::raw_schema_fingerprint()?)
        {
            return Err(ArchiveError::InvalidInput(
                "logical commit object coverage or schema fingerprint",
            ));
        }
        let expected_path = logical_object_relative_path(&commit, object.sha256()?)?;
        if object.relative_path != expected_path {
            return Err(ArchiveError::InvalidInput(
                "logical commit object path is not content-addressed",
            ));
        }
        Ok(Self {
            schema: RAW_LOGICAL_COMMIT_SCHEMA_V3.to_owned(),
            producer_build_id,
            created_at_micros: created_at.unix_micros(),
            commit,
            object,
        })
    }

    #[must_use]
    pub const fn commit(&self) -> &LogicalCommitDescriptorV3 {
        &self.commit
    }

    #[must_use]
    pub const fn object(&self) -> &LogicalObjectDescriptorV3 {
        &self.object
    }

    pub fn manifest_sha256(&self) -> Result<[u8; 32], ArchiveError> {
        Ok(manifest::sha256(&manifest::canonical_json(self)?))
    }

    #[must_use]
    pub const fn created_at_micros(&self) -> i64 {
        self.created_at_micros
    }
}

pub fn parse_logical_commit_manifest(
    bytes: &[u8],
) -> Result<LogicalCommitManifestV3, ArchiveError> {
    let manifest_size = u64::try_from(bytes.len()).map_err(|_| {
        ArchiveError::ManifestVerification("raw V3 logical commit exceeds address space")
    })?;
    if bytes.is_empty() || manifest_size > RAW_ARCHIVE_MAXIMUM_LOGICAL_MANIFEST_BYTES {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 logical commit size is invalid",
        ));
    }
    let wire: LogicalCommitManifestWireV3 = serde_json::from_slice(bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 logical commit JSON"))?;
    if wire.schema != RAW_LOGICAL_COMMIT_SCHEMA_V3
        || wire.commit.cursor_policy != RAW_V3_CURSOR_POLICY
        || KnownTime::from_unix_micros(wire.created_at_micros).is_err()
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 logical commit schema, cursor policy, or time is invalid",
        ));
    }
    if wire.object.schema_fingerprint_sha256 != hex::encode(schema::raw_schema_fingerprint()?) {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 logical commit schema fingerprint is invalid",
        ));
    }
    let commit = LogicalCommitDescriptorV3::try_new(
        ChainId::new(wire.commit.chain_id).map_err(|_| {
            ArchiveError::ManifestVerification("invalid raw V3 logical commit chain")
        })?,
        SourceId::new(wire.commit.source_id).map_err(|_| {
            ArchiveError::ManifestVerification("invalid raw V3 logical commit source")
        })?,
        wire.commit.source_version,
        wire.commit.observation_class,
        wire.commit.cursor_epoch,
        wire.commit.start_offset,
        wire.commit.end_offset,
        wire.commit.first_local_sequence,
        wire.commit.last_local_sequence,
        wire.commit.first_received_wall_micros,
        wire.commit.last_received_wall_micros,
        wire.commit.parser_schema_version,
        manifest::parse_hash(&wire.commit.spool_manifest_blake3)?,
        manifest::parse_hash(&wire.commit.spool_segment_blake3)?,
        manifest::parse_hash(&wire.commit.rolling_content_sha256)?,
    )?;
    let object = LogicalObjectDescriptorV3::try_new(
        PathBuf::from(wire.object.relative_path),
        manifest::parse_hash(&wire.object.sha256)?,
        wire.object.size_bytes,
        wire.object.row_count,
    )?;
    let created_at = KnownTime::from_unix_micros(wire.created_at_micros)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 logical commit time"))?;
    let reconstructed =
        LogicalCommitManifestV3::try_new(wire.producer_build_id, created_at, commit, object)?;
    if reconstructed.created_at_micros != wire.created_at_micros
        || manifest::canonical_json(&reconstructed)? != bytes
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 logical commit canonical bytes are invalid",
        ));
    }
    Ok(reconstructed)
}

pub fn logical_commit_domain_hash(
    commit: &LogicalCommitManifestV3,
) -> Result<[u8; 32], ArchiveError> {
    domain_hash(
        LOGICAL_COMMIT_HASH_DOMAIN_V3,
        &manifest::canonical_json(commit)?,
    )
}

pub(crate) fn logical_object_relative_path(
    commit: &LogicalCommitDescriptorV3,
    object_sha256: [u8; 32],
) -> Result<String, ArchiveError> {
    let chain = commit.chain_id()?;
    let source = commit.source_id()?;
    let partition = commit.partition()?;
    let path = PathBuf::from(format!(
        "chain={}",
        manifest::encoded_component(chain.as_str())
    ))
    .join(format!("dataset={RAW_BYTE_DATASET_V3}"))
    .join(format!(
        "source={}",
        manifest::encoded_component(source.as_str())
    ))
    .join(partition)
    .join("objects")
    .join(format!(
        "epoch={}",
        manifest::encoded_component(&commit.cursor_epoch)
    ))
    .join(format!(
        "sequences={}-{}",
        commit.first_local_sequence, commit.last_local_sequence
    ))
    .join(format!(
        "offsets={}-{}",
        commit.start_offset, commit.end_offset
    ))
    .join(format!("part-{}.parquet", hex::encode(object_sha256)));
    checked_relative_string(&path)
}

pub(crate) fn journal_file_identity(generation: u64) -> Result<String, ArchiveError> {
    if generation == 0 {
        return Err(ArchiveError::InvalidInput(
            "journal generation must be nonzero",
        ));
    }
    Ok(format!("{RAW_V3_JOURNAL_FILE_IDENTITY_PREFIX}{generation}"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackedLogicalInputV3 {
    original_schema: String,
    manifest_id: String,
    canonical_manifest_json: String,
    manifest_sha256: String,
    chain_id: String,
    source_id: String,
    partition: String,
    object_sha256: String,
    first_local_sequence: u64,
    last_local_sequence: u64,
    row_slice_start: u64,
    row_count: u64,
    cursor_epoch: String,
    start_offset: u64,
    end_offset: u64,
    rolling_content_sha256: String,
}

impl PackedLogicalInputV3 {
    pub fn try_new_v2(
        canonical_manifest_bytes: Vec<u8>,
        manifest_sha256: [u8; 32],
        row_slice_start: u64,
    ) -> Result<Self, ArchiveError> {
        let evidence = super::raw_v2::validate_embedded_manifest_v2(
            canonical_manifest_bytes,
            manifest_sha256,
        )?;
        if row_slice_start.checked_add(evidence.row_count).is_none() {
            return Err(ArchiveError::InvalidInput(
                "packed logical row slice overflows",
            ));
        }
        Ok(Self {
            original_schema: "raw-v2".to_owned(),
            manifest_id: evidence.manifest_id.as_str().to_owned(),
            canonical_manifest_json: evidence.canonical_manifest_json,
            manifest_sha256: hex::encode(manifest_sha256),
            chain_id: evidence.chain_id.as_str().to_owned(),
            source_id: evidence.source_id.as_str().to_owned(),
            partition: evidence.partition,
            object_sha256: hex::encode(evidence.object_sha256),
            first_local_sequence: evidence.first_local_sequence,
            last_local_sequence: evidence.last_local_sequence,
            row_slice_start,
            row_count: evidence.row_count,
            cursor_epoch: evidence.cursor_epoch,
            start_offset: evidence.start_offset,
            end_offset: evidence.end_offset,
            rolling_content_sha256: hex::encode(evidence.rolling_content_sha256),
        })
    }

    pub fn try_new_v3(
        canonical_manifest_bytes: Vec<u8>,
        manifest_sha256: [u8; 32],
        row_slice_start: u64,
    ) -> Result<Self, ArchiveError> {
        if manifest::sha256(&canonical_manifest_bytes) != manifest_sha256 {
            return Err(ArchiveError::ManifestVerification(
                "packed raw V3 logical commit hash does not match canonical bytes",
            ));
        }
        let commit = parse_logical_commit_manifest(&canonical_manifest_bytes)?;
        let row_count = commit.object.row_count;
        if row_slice_start.checked_add(row_count).is_none() {
            return Err(ArchiveError::InvalidInput(
                "packed logical row slice overflows",
            ));
        }
        let canonical_manifest_json =
            String::from_utf8(canonical_manifest_bytes).map_err(|_| {
                ArchiveError::ManifestVerification("raw V3 logical commit is not UTF-8")
            })?;
        Ok(Self {
            original_schema: "raw-v3".to_owned(),
            manifest_id: super::manifest::manifest_id(manifest_sha256)?
                .as_str()
                .to_owned(),
            canonical_manifest_json,
            manifest_sha256: hex::encode(manifest_sha256),
            chain_id: commit.commit.chain_id.clone(),
            source_id: commit.commit.source_id.clone(),
            partition: commit.commit.partition()?,
            object_sha256: commit.object.sha256.clone(),
            first_local_sequence: commit.commit.first_local_sequence,
            last_local_sequence: commit.commit.last_local_sequence,
            row_slice_start,
            row_count,
            cursor_epoch: commit.commit.cursor_epoch.clone(),
            start_offset: commit.commit.start_offset,
            end_offset: commit.commit.end_offset,
            rolling_content_sha256: commit.commit.rolling_content_sha256.clone(),
        })
    }

    fn row_slice_end(&self) -> Result<u64, ArchiveError> {
        self.row_slice_start
            .checked_add(self.row_count)
            .ok_or(ArchiveError::InvalidInput(
                "packed logical row slice overflows",
            ))
    }

    pub fn manifest_sha256(&self) -> Result<[u8; 32], ArchiveError> {
        manifest::parse_hash(&self.manifest_sha256)
    }

    #[must_use]
    pub const fn first_local_sequence(&self) -> u64 {
        self.first_local_sequence
    }

    #[must_use]
    pub const fn last_local_sequence(&self) -> u64 {
        self.last_local_sequence
    }

    #[must_use]
    pub const fn row_slice_start(&self) -> u64 {
        self.row_slice_start
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }

    #[must_use]
    pub fn cursor_epoch(&self) -> &str {
        &self.cursor_epoch
    }

    #[must_use]
    pub fn canonical_manifest_json(&self) -> &str {
        &self.canonical_manifest_json
    }

    pub fn object_sha256(&self) -> Result<[u8; 32], ArchiveError> {
        manifest::parse_hash(&self.object_sha256)
    }

    #[must_use]
    pub fn manifest_id(&self) -> &str {
        &self.manifest_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackedObjectDescriptorV3 {
    relative_path: String,
    sha256: String,
    size_bytes: u64,
    row_count: u64,
    schema_fingerprint_sha256: String,
}

impl PackedObjectDescriptorV3 {
    pub fn try_new(
        relative_path: PathBuf,
        sha256: [u8; 32],
        size_bytes: u64,
        row_count: u64,
    ) -> Result<Self, ArchiveError> {
        if size_bytes == 0 || size_bytes > RAW_ARCHIVE_MAXIMUM_DATA_PACK_BYTES || row_count == 0 {
            return Err(ArchiveError::InvalidInput(
                "packed object size and row count must be nonzero",
            ));
        }
        Ok(Self {
            relative_path: checked_relative_string(&relative_path)?,
            sha256: hex::encode(sha256),
            size_bytes,
            row_count,
            schema_fingerprint_sha256: hex::encode(schema::raw_schema_fingerprint()?),
        })
    }

    #[must_use]
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    pub fn sha256(&self) -> Result<[u8; 32], ArchiveError> {
        manifest::parse_hash(&self.sha256)
    }

    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawPackManifestV3 {
    schema: String,
    chain_id: String,
    source_id: String,
    dataset: String,
    partition: String,
    created_at_micros: i64,
    first_local_sequence: u64,
    last_local_sequence: u64,
    logical_manifest_count: u64,
    combined_rolling_content_sha256: String,
    inputs: Vec<PackedLogicalInputV3>,
    object: PackedObjectDescriptorV3,
}

impl RawPackManifestV3 {
    pub fn try_new(
        inputs: Vec<PackedLogicalInputV3>,
        object: PackedObjectDescriptorV3,
        created_at: KnownTime,
    ) -> Result<Self, ArchiveError> {
        if inputs.len() < 2 || inputs.len() > RAW_ARCHIVE_MAXIMUM_PACK_LOGICAL_INPUTS {
            return Err(ArchiveError::InvalidInput("raw pack logical input count"));
        }
        let chain_id = inputs[0].chain_id.clone();
        let source_id = inputs[0].source_id.clone();
        let partition = inputs[0].partition.clone();
        let mut manifest_ids = BTreeSet::new();
        let mut manifest_hashes = BTreeSet::new();
        let mut seen_epochs = BTreeSet::new();
        let mut current_epoch: Option<&str> = None;
        let mut prior_end_offset = None;
        let mut expected_sequence = inputs[0].first_local_sequence;
        let mut expected_row_start = 0_u64;
        let mut embedded_bytes = 0_usize;
        for (index, input) in inputs.iter().enumerate() {
            if input.first_local_sequence != expected_sequence
                || input.row_slice_start != expected_row_start
                || input.chain_id != chain_id
                || input.source_id != source_id
                || input.partition != partition
                || !manifest_ids.insert(input.manifest_id.as_str())
                || !manifest_hashes.insert(input.manifest_sha256.as_str())
            {
                return Err(ArchiveError::InvalidInput(
                    "raw pack inputs must be contiguous with unique receipts",
                ));
            }
            embedded_bytes = embedded_bytes
                .checked_add(input.canonical_manifest_json.len())
                .ok_or(ArchiveError::InvalidInput(
                    "raw pack embedded manifest bytes overflow",
                ))?;
            let embedded_bytes_u64 = u64::try_from(embedded_bytes).map_err(|_| {
                ArchiveError::InvalidInput("raw pack embedded manifest bytes exceed u64")
            })?;
            if embedded_bytes_u64 > RAW_ARCHIVE_MAXIMUM_EMBEDDED_PACK_MANIFEST_BYTES {
                return Err(ArchiveError::InvalidInput(
                    "raw pack embedded manifest bytes exceed the global bound",
                ));
            }
            if current_epoch != Some(input.cursor_epoch.as_str()) {
                if !seen_epochs.insert(input.cursor_epoch.as_str()) {
                    return Err(ArchiveError::InvalidInput(
                        "raw pack cursor epoch recurs after a later epoch",
                    ));
                }
                current_epoch = Some(input.cursor_epoch.as_str());
                prior_end_offset = None;
            }
            if prior_end_offset.is_some_and(|end| input.start_offset <= end) {
                return Err(ArchiveError::InvalidInput(
                    "raw pack native cursor ranges overlap or regress",
                ));
            }
            prior_end_offset = Some(input.end_offset);
            if index + 1 < inputs.len() {
                expected_sequence =
                    input
                        .last_local_sequence
                        .checked_add(1)
                        .ok_or(ArchiveError::InvalidInput(
                            "raw pack local sequence overflows",
                        ))?;
            }
            expected_row_start = input.row_slice_end()?;
        }
        if expected_row_start != object.row_count {
            return Err(ArchiveError::InvalidInput(
                "raw pack row slices do not cover the output exactly",
            ));
        }
        let expected_object_path = format!("{partition}/packs/pack-{}.parquet", object.sha256);
        if object.relative_path != expected_object_path {
            return Err(ArchiveError::InvalidInput(
                "raw pack object path is not content-addressed",
            ));
        }
        let first_local_sequence = inputs[0].first_local_sequence;
        let last_local_sequence = inputs
            .last()
            .ok_or(ArchiveError::InvalidInput("raw pack inputs are empty"))?
            .last_local_sequence;
        let logical_manifest_count = u64::try_from(inputs.len())
            .map_err(|_| ArchiveError::InvalidInput("raw pack input count exceeds u64"))?;
        let combined_rolling_content_sha256 = combined_pack_hash(&inputs)?;
        Ok(Self {
            schema: RAW_PACK_MANIFEST_SCHEMA_V3.to_owned(),
            chain_id,
            source_id,
            dataset: RAW_BYTE_DATASET_V3.to_owned(),
            partition,
            created_at_micros: created_at.unix_micros(),
            first_local_sequence,
            last_local_sequence,
            logical_manifest_count,
            combined_rolling_content_sha256: hex::encode(combined_rolling_content_sha256),
            inputs,
            object,
        })
    }

    #[must_use]
    pub const fn first_local_sequence(&self) -> u64 {
        self.first_local_sequence
    }

    #[must_use]
    pub const fn last_local_sequence(&self) -> u64 {
        self.last_local_sequence
    }

    #[must_use]
    pub const fn logical_manifest_count(&self) -> u64 {
        self.logical_manifest_count
    }

    #[must_use]
    pub fn inputs(&self) -> &[PackedLogicalInputV3] {
        &self.inputs
    }

    #[must_use]
    pub const fn object(&self) -> &PackedObjectDescriptorV3 {
        &self.object
    }

    #[must_use]
    pub fn partition(&self) -> &str {
        &self.partition
    }

    #[must_use]
    pub const fn created_at_micros(&self) -> i64 {
        self.created_at_micros
    }

    pub fn chain_id(&self) -> Result<ChainId, ArchiveError> {
        ChainId::new(self.chain_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("invalid raw pack chain"))
    }

    pub fn source_id(&self) -> Result<SourceId, ArchiveError> {
        SourceId::new(self.source_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("invalid raw pack source"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceLeafPageV3 {
    schema: String,
    chain_id: String,
    source_id: String,
    dataset: String,
    first_local_sequence: u64,
    last_local_sequence: u64,
    row_count: u64,
    logical_manifest_count: u64,
    entries: Vec<SequenceLeafEntryV3>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceLeafPageWireV3 {
    schema: String,
    chain_id: String,
    source_id: String,
    dataset: String,
    first_local_sequence: u64,
    last_local_sequence: u64,
    row_count: u64,
    logical_manifest_count: u64,
    entries: Vec<SequenceLeafEntryWireV3>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceLeafEntryWireV3 {
    first_local_sequence: u64,
    last_local_sequence: u64,
    partition: String,
    object_size_bytes: u64,
    row_count: u64,
    logical_manifest_count: u64,
    storage: SequenceStorageWireV3,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum SequenceStorageWireV3 {
    Logical {
        manifest_relative_path: String,
        manifest_sha256: String,
    },
    Packed {
        pack_manifest_relative_path: String,
        pack_manifest_sha256: String,
    },
}

pub fn parse_sequence_leaf_page(bytes: &[u8]) -> Result<SequenceLeafPageV3, ArchiveError> {
    let page_size = u64::try_from(bytes.len())
        .map_err(|_| ArchiveError::ManifestVerification("raw V3 sequence leaf page too large"))?;
    if bytes.is_empty() || page_size > RAW_ARCHIVE_MAXIMUM_SEQUENCE_PAGE_BYTES {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 sequence leaf page size is invalid",
        ));
    }
    let wire: SequenceLeafPageWireV3 = serde_json::from_slice(bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 sequence leaf JSON"))?;
    if wire.schema != RAW_SEQUENCE_LEAF_SCHEMA_V3 || wire.dataset != RAW_BYTE_DATASET_V3 {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 sequence leaf schema or dataset is invalid",
        ));
    }
    let chain_id = ChainId::new(wire.chain_id)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 sequence leaf chain"))?;
    let source_id = SourceId::new(wire.source_id)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 sequence leaf source"))?;
    let mut entries = Vec::with_capacity(wire.entries.len().min(MAX_SEQUENCE_LEAF_ENTRIES));
    for entry in wire.entries {
        let reconstructed = match entry.storage {
            SequenceStorageWireV3::Logical {
                manifest_relative_path,
                manifest_sha256,
            } => SequenceLeafEntryV3::try_new_logical(
                entry.first_local_sequence,
                entry.last_local_sequence,
                manifest_relative_path,
                manifest::parse_hash(&manifest_sha256)?,
                entry.object_size_bytes,
                entry.row_count,
                entry.partition,
            )?,
            SequenceStorageWireV3::Packed {
                pack_manifest_relative_path,
                pack_manifest_sha256,
            } => SequenceLeafEntryV3::try_new_packed(
                entry.first_local_sequence,
                entry.last_local_sequence,
                pack_manifest_relative_path,
                manifest::parse_hash(&pack_manifest_sha256)?,
                entry.object_size_bytes,
                entry.row_count,
                entry.logical_manifest_count,
                entry.partition,
            )?,
        };
        if reconstructed.logical_manifest_count != entry.logical_manifest_count {
            return Err(ArchiveError::ManifestVerification(
                "raw V3 sequence leaf logical count is invalid",
            ));
        }
        entries.push(reconstructed);
    }
    let page = SequenceLeafPageV3::try_new(chain_id, source_id, entries)?;
    if page.first_local_sequence != wire.first_local_sequence
        || page.last_local_sequence != wire.last_local_sequence
        || page.row_count != wire.row_count
        || page.logical_manifest_count != wire.logical_manifest_count
        || manifest::canonical_json(&page)? != bytes
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 sequence leaf aggregate or canonical bytes are invalid",
        ));
    }
    Ok(page)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceNodeRefWireV3 {
    chain_id: String,
    source_id: String,
    depth: u8,
    first_local_sequence: u64,
    last_local_sequence: u64,
    row_count: u64,
    logical_manifest_count: u64,
    locator: SequencePageLocatorWireV3,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum SequencePageLocatorWireV3 {
    Journal {
        generation: u64,
        file_identity: String,
        record_sequence: u64,
        payload_offset: u64,
        payload_length: u64,
        page_domain_sha256: String,
    },
    IndexPack {
        pack_relative_path: String,
        pack_sha256: String,
        payload_offset: u64,
        payload_length: u64,
        page_domain_sha256: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceInternalPageWireV3 {
    schema: String,
    chain_id: String,
    source_id: String,
    dataset: String,
    depth: u8,
    first_local_sequence: u64,
    last_local_sequence: u64,
    row_count: u64,
    logical_manifest_count: u64,
    children: Vec<SequenceNodeRefWireV3>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptHintPageWireV3 {
    schema: String,
    chain_id: String,
    source_id: String,
    dataset: String,
    authoritative: bool,
    entries: Vec<ReceiptHintEntryWireV3>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptHintEntryWireV3 {
    manifest_sha256: String,
    first_local_sequence: u64,
    last_local_sequence: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootBundleWireV3 {
    schema: String,
    chain_id: String,
    source_id: String,
    dataset: String,
    generation: u64,
    created_at_micros: i64,
    previous_root_sha256: Option<String>,
    journal_prefix: JournalPrefixRefWireV3,
    sequence_root: SequenceNodeRefWireV3,
    head_local_sequence: u64,
    logical_manifest_count: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalPrefixRefWireV3 {
    generation: u64,
    file_identity: String,
    relative_path: String,
    committed_prefix_length: u64,
    committed_record_count: u64,
    committed_prefix_sha256: String,
    root_record_sequence: u64,
    root_first_local_sequence: u64,
    root_last_local_sequence: u64,
    root_row_count: u64,
    root_logical_manifest_count: u64,
    root_page_domain_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackManifestWireV3 {
    schema: String,
    chain_id: String,
    source_id: String,
    dataset: String,
    partition: String,
    created_at_micros: i64,
    first_local_sequence: u64,
    last_local_sequence: u64,
    logical_manifest_count: u64,
    combined_rolling_content_sha256: String,
    inputs: Vec<PackedLogicalInputWireV3>,
    object: PackedObjectDescriptorWireV3,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackedLogicalInputWireV3 {
    original_schema: String,
    manifest_id: String,
    canonical_manifest_json: String,
    manifest_sha256: String,
    chain_id: String,
    source_id: String,
    partition: String,
    object_sha256: String,
    first_local_sequence: u64,
    last_local_sequence: u64,
    row_slice_start: u64,
    row_count: u64,
    cursor_epoch: String,
    start_offset: u64,
    end_offset: u64,
    rolling_content_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackedObjectDescriptorWireV3 {
    relative_path: String,
    sha256: String,
    size_bytes: u64,
    row_count: u64,
    schema_fingerprint_sha256: String,
}

fn parse_sequence_page_locator(
    wire: SequencePageLocatorWireV3,
) -> Result<SequencePageLocatorV3, ArchiveError> {
    match wire {
        SequencePageLocatorWireV3::Journal {
            generation,
            file_identity,
            record_sequence,
            payload_offset,
            payload_length,
            page_domain_sha256,
        } => SequencePageLocatorV3::journal(
            generation,
            &file_identity,
            record_sequence,
            payload_offset,
            payload_length,
            manifest::parse_hash(&page_domain_sha256)?,
        ),
        SequencePageLocatorWireV3::IndexPack {
            pack_relative_path,
            pack_sha256,
            payload_offset,
            payload_length,
            page_domain_sha256,
        } => {
            let reconstructed = SequencePageLocatorV3::index_pack(
                manifest::parse_hash(&pack_sha256)?,
                payload_offset,
                payload_length,
                manifest::parse_hash(&page_domain_sha256)?,
            )?;
            let SequencePageLocatorV3::IndexPack {
                pack_relative_path: expected_path,
                ..
            } = &reconstructed
            else {
                return Err(ArchiveError::ManifestVerification(
                    "index-pack locator reconstruction failed",
                ));
            };
            if expected_path != &pack_relative_path {
                return Err(ArchiveError::ManifestVerification(
                    "index-pack locator path is not content-addressed",
                ));
            }
            Ok(reconstructed)
        }
    }
}

fn parse_sequence_node_ref(wire: SequenceNodeRefWireV3) -> Result<SequenceNodeRefV3, ArchiveError> {
    let chain_id = ChainId::new(wire.chain_id)
        .map_err(|_| ArchiveError::ManifestVerification("invalid sequence node chain"))?;
    let source_id = SourceId::new(wire.source_id)
        .map_err(|_| ArchiveError::ManifestVerification("invalid sequence node source"))?;
    let expected_rows = sequence_span(wire.first_local_sequence, wire.last_local_sequence)?;
    if wire.row_count != expected_rows
        || wire.logical_manifest_count == 0
        || wire.depth > RAW_ARCHIVE_MAXIMUM_SEQUENCE_TREE_DEPTH
    {
        return Err(ArchiveError::ManifestVerification(
            "sequence node coverage, count, or depth is invalid",
        ));
    }
    Ok(SequenceNodeRefV3 {
        chain_id: chain_id.as_str().to_owned(),
        source_id: source_id.as_str().to_owned(),
        depth: wire.depth,
        first_local_sequence: wire.first_local_sequence,
        last_local_sequence: wire.last_local_sequence,
        row_count: wire.row_count,
        logical_manifest_count: wire.logical_manifest_count,
        locator: parse_sequence_page_locator(wire.locator)?,
    })
}

pub fn parse_sequence_internal_page(bytes: &[u8]) -> Result<SequenceInternalPageV3, ArchiveError> {
    let page_size = u64::try_from(bytes.len()).map_err(|_| {
        ArchiveError::ManifestVerification("raw V3 sequence internal page too large")
    })?;
    if bytes.is_empty() || page_size > RAW_ARCHIVE_MAXIMUM_SEQUENCE_PAGE_BYTES {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 sequence internal page size is invalid",
        ));
    }
    let wire: SequenceInternalPageWireV3 = serde_json::from_slice(bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 sequence internal JSON"))?;
    if wire.schema != RAW_SEQUENCE_INTERNAL_SCHEMA_V3 || wire.dataset != RAW_BYTE_DATASET_V3 {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 sequence internal schema or dataset is invalid",
        ));
    }
    let chain_id = ChainId::new(wire.chain_id).map_err(|_| {
        ArchiveError::ManifestVerification("invalid raw V3 sequence internal chain")
    })?;
    let source_id = SourceId::new(wire.source_id).map_err(|_| {
        ArchiveError::ManifestVerification("invalid raw V3 sequence internal source")
    })?;
    let mut children = Vec::with_capacity(wire.children.len().min(MAX_SEQUENCE_INTERNAL_CHILDREN));
    for child in wire.children {
        children.push(parse_sequence_node_ref(child)?);
    }
    let page = SequenceInternalPageV3::try_new(chain_id, source_id, wire.depth, children)?;
    if page.first_local_sequence != wire.first_local_sequence
        || page.last_local_sequence != wire.last_local_sequence
        || page.row_count != wire.row_count
        || page.logical_manifest_count != wire.logical_manifest_count
        || manifest::canonical_json(&page)? != bytes
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 sequence internal aggregate or canonical bytes are invalid",
        ));
    }
    Ok(page)
}

pub fn parse_receipt_hint_page(bytes: &[u8]) -> Result<ReceiptHintPageV3, ArchiveError> {
    let page_size = u64::try_from(bytes.len())
        .map_err(|_| ArchiveError::ManifestVerification("raw V3 receipt hint page too large"))?;
    if bytes.is_empty() || page_size > RAW_ARCHIVE_MAXIMUM_SEQUENCE_PAGE_BYTES {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 receipt hint page size is invalid",
        ));
    }
    let wire: ReceiptHintPageWireV3 = serde_json::from_slice(bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 receipt hint JSON"))?;
    if wire.schema != RAW_RECEIPT_HINT_PAGE_SCHEMA_V3
        || wire.dataset != RAW_BYTE_DATASET_V3
        || wire.authoritative
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 receipt hint schema, dataset, or authority flag is invalid",
        ));
    }
    let chain_id = ChainId::new(wire.chain_id)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 receipt hint chain"))?;
    let source_id = SourceId::new(wire.source_id)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 receipt hint source"))?;
    let mut entries = Vec::with_capacity(wire.entries.len().min(MAX_RECEIPT_HINT_ENTRIES));
    for entry in wire.entries {
        entries.push(ReceiptHintEntryV3::try_new(
            manifest::parse_hash(&entry.manifest_sha256)?,
            entry.first_local_sequence,
            entry.last_local_sequence,
        )?);
    }
    let page = ReceiptHintPageV3::try_new(chain_id, source_id, entries)?;
    if manifest::canonical_json(&page)? != bytes {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 receipt hint canonical bytes are invalid",
        ));
    }
    Ok(page)
}

pub fn parse_root_bundle(bytes: &[u8]) -> Result<RootBundleV3, ArchiveError> {
    let wire: RootBundleWireV3 = serde_json::from_slice(bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 root bundle JSON"))?;
    if wire.schema != RAW_ROOT_BUNDLE_SCHEMA_V3 || wire.dataset != RAW_BYTE_DATASET_V3 {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 root bundle schema or dataset is invalid",
        ));
    }
    let created_at = KnownTime::from_unix_micros(wire.created_at_micros)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 root bundle time"))?;
    let previous = wire
        .previous_root_sha256
        .as_deref()
        .map(manifest::parse_hash)
        .transpose()?;
    let sequence_root = parse_sequence_node_ref(wire.sequence_root)?;
    let journal_prefix = journal_prefix_from_wire(wire.journal_prefix, &sequence_root)?;
    let reconstructed = RootBundleV3::from_prefix_and_root(
        ChainId::new(wire.chain_id)
            .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 root bundle chain"))?,
        SourceId::new(wire.source_id)
            .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 root bundle source"))?,
        wire.generation,
        previous,
        &journal_prefix,
        &sequence_root,
        created_at,
    )?;
    if reconstructed.head_local_sequence != wire.head_local_sequence
        || reconstructed.logical_manifest_count != wire.logical_manifest_count
        || reconstructed.created_at_micros != wire.created_at_micros
        || manifest::canonical_json(&reconstructed)? != bytes
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 root bundle aggregate or canonical bytes are invalid",
        ));
    }
    Ok(reconstructed)
}

fn journal_prefix_from_wire(
    wire: JournalPrefixRefWireV3,
    root: &SequenceNodeRefV3,
) -> Result<JournalPrefixRefV3, ArchiveError> {
    validate_text(&wire.file_identity, "journal file identity")?;
    let relative_path = checked_relative_string(Path::new(&wire.relative_path))?;
    manifest::parse_hash(&wire.committed_prefix_sha256)?;
    manifest::parse_hash(&wire.root_page_domain_sha256)?;
    if wire.generation == 0
        || wire.committed_prefix_length == 0
        || wire.committed_record_count == 0
        || wire.root_record_sequence == 0
        || wire.root_record_sequence > wire.committed_record_count
        || wire.root_first_local_sequence != 1
        || wire.root_first_local_sequence != root.first_local_sequence
        || wire.root_last_local_sequence != root.last_local_sequence
        || wire.root_row_count != root.row_count
        || wire.root_logical_manifest_count != root.logical_manifest_count
        || wire.generation
            != match &root.locator {
                SequencePageLocatorV3::Journal { generation, .. } => *generation,
                SequencePageLocatorV3::IndexPack { .. } => wire.generation,
            }
    {
        return Err(ArchiveError::ManifestVerification(
            "journal prefix does not authenticate the sequence root",
        ));
    }
    if let SequencePageLocatorV3::Journal {
        file_identity,
        page_domain_sha256,
        ..
    } = &root.locator
        && (file_identity != &wire.file_identity
            || page_domain_sha256 != &wire.root_page_domain_sha256)
    {
        return Err(ArchiveError::ManifestVerification(
            "journal prefix page identity does not match the sequence root",
        ));
    }
    Ok(JournalPrefixRefV3 {
        generation: wire.generation,
        file_identity: wire.file_identity,
        relative_path,
        committed_prefix_length: wire.committed_prefix_length,
        committed_record_count: wire.committed_record_count,
        committed_prefix_sha256: wire.committed_prefix_sha256,
        root_record_sequence: wire.root_record_sequence,
        root_first_local_sequence: wire.root_first_local_sequence,
        root_last_local_sequence: wire.root_last_local_sequence,
        root_row_count: wire.root_row_count,
        root_logical_manifest_count: wire.root_logical_manifest_count,
        root_page_domain_sha256: wire.root_page_domain_sha256,
    })
}

pub fn parse_pack_manifest(bytes: &[u8]) -> Result<RawPackManifestV3, ArchiveError> {
    let wire: RawPackManifestWireV3 = serde_json::from_slice(bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 pack manifest JSON"))?;
    if wire.schema != RAW_PACK_MANIFEST_SCHEMA_V3 || wire.dataset != RAW_BYTE_DATASET_V3 {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 pack schema or dataset is invalid",
        ));
    }
    let created_at = KnownTime::from_unix_micros(wire.created_at_micros)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 pack time"))?;
    if wire.object.schema_fingerprint_sha256 != hex::encode(schema::raw_schema_fingerprint()?) {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 pack schema fingerprint is invalid",
        ));
    }
    let object = PackedObjectDescriptorV3::try_new(
        PathBuf::from(wire.object.relative_path),
        manifest::parse_hash(&wire.object.sha256)?,
        wire.object.size_bytes,
        wire.object.row_count,
    )?;
    let mut inputs = Vec::with_capacity(wire.inputs.len());
    for input in wire.inputs {
        let manifest_bytes = input.canonical_manifest_json.into_bytes();
        let manifest_sha256 = manifest::parse_hash(&input.manifest_sha256)?;
        let reconstructed = match input.original_schema.as_str() {
            "raw-v2" => PackedLogicalInputV3::try_new_v2(
                manifest_bytes,
                manifest_sha256,
                input.row_slice_start,
            )?,
            "raw-v3" => PackedLogicalInputV3::try_new_v3(
                manifest_bytes,
                manifest_sha256,
                input.row_slice_start,
            )?,
            _ => {
                return Err(ArchiveError::ManifestVerification(
                    "raw V3 pack input original schema is unsupported",
                ));
            }
        };
        if reconstructed.manifest_id != input.manifest_id
            || reconstructed.chain_id != input.chain_id
            || reconstructed.source_id != input.source_id
            || reconstructed.partition != input.partition
            || reconstructed.object_sha256 != input.object_sha256
            || reconstructed.first_local_sequence != input.first_local_sequence
            || reconstructed.last_local_sequence != input.last_local_sequence
            || reconstructed.row_count != input.row_count
            || reconstructed.cursor_epoch != input.cursor_epoch
            || reconstructed.start_offset != input.start_offset
            || reconstructed.end_offset != input.end_offset
            || reconstructed.rolling_content_sha256 != input.rolling_content_sha256
        {
            return Err(ArchiveError::ManifestVerification(
                "raw V3 pack input is not derived from its embedded manifest",
            ));
        }
        inputs.push(reconstructed);
    }
    let pack = RawPackManifestV3::try_new(inputs, object, created_at)?;
    if pack.first_local_sequence != wire.first_local_sequence
        || pack.last_local_sequence != wire.last_local_sequence
        || pack.logical_manifest_count != wire.logical_manifest_count
        || pack.combined_rolling_content_sha256 != wire.combined_rolling_content_sha256
        || pack.chain_id != wire.chain_id
        || pack.source_id != wire.source_id
        || pack.partition != wire.partition
        || manifest::canonical_json(&pack)? != bytes
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 pack aggregate or canonical bytes are invalid",
        ));
    }
    Ok(pack)
}

impl SequenceLeafPageV3 {
    pub fn try_new(
        chain_id: ChainId,
        source_id: SourceId,
        entries: Vec<SequenceLeafEntryV3>,
    ) -> Result<Self, ArchiveError> {
        if entries.is_empty() || entries.len() > MAX_SEQUENCE_LEAF_ENTRIES {
            return Err(ArchiveError::InvalidInput(
                "sequence leaf page fanout must be bounded and nonzero",
            ));
        }
        for pair in entries.windows(2) {
            let expected_next =
                pair[0]
                    .last_local_sequence
                    .checked_add(1)
                    .ok_or(ArchiveError::InvalidInput(
                        "sequence leaf coverage overflows",
                    ))?;
            if pair[1].first_local_sequence != expected_next {
                return Err(ArchiveError::InvalidInput(
                    "sequence leaf entries must be exactly contiguous",
                ));
            }
        }
        let first_local_sequence = entries[0].first_local_sequence;
        let last_local_sequence = entries
            .last()
            .ok_or(ArchiveError::InvalidInput("sequence leaf page is empty"))?
            .last_local_sequence;
        let row_count = checked_sum(entries.iter().map(|entry| entry.row_count))?;
        if row_count != sequence_span(first_local_sequence, last_local_sequence)? {
            return Err(ArchiveError::InvalidInput(
                "sequence leaf page row coverage is not exact",
            ));
        }
        let logical_manifest_count =
            checked_sum(entries.iter().map(|entry| entry.logical_manifest_count))?;
        Ok(Self {
            schema: RAW_SEQUENCE_LEAF_SCHEMA_V3.to_owned(),
            chain_id: chain_id.as_str().to_owned(),
            source_id: source_id.as_str().to_owned(),
            dataset: RAW_BYTE_DATASET_V3.to_owned(),
            first_local_sequence,
            last_local_sequence,
            row_count,
            logical_manifest_count,
            entries,
        })
    }

    #[must_use]
    pub const fn first_local_sequence(&self) -> u64 {
        self.first_local_sequence
    }

    #[must_use]
    pub const fn last_local_sequence(&self) -> u64 {
        self.last_local_sequence
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }

    #[must_use]
    pub fn entries(&self) -> &[SequenceLeafEntryV3] {
        &self.entries
    }

    pub fn chain_id(&self) -> Result<ChainId, ArchiveError> {
        ChainId::new(self.chain_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("invalid sequence leaf chain"))
    }

    pub fn source_id(&self) -> Result<SourceId, ArchiveError> {
        SourceId::new(self.source_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("invalid sequence leaf source"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum SequencePageLocatorV3 {
    Journal {
        generation: u64,
        file_identity: String,
        record_sequence: u64,
        payload_offset: u64,
        payload_length: u64,
        page_domain_sha256: String,
    },
    IndexPack {
        pack_relative_path: String,
        pack_sha256: String,
        payload_offset: u64,
        payload_length: u64,
        page_domain_sha256: String,
    },
}

impl SequencePageLocatorV3 {
    fn journal(
        generation: u64,
        file_identity: &str,
        record_sequence: u64,
        payload_offset: u64,
        payload_length: u64,
        page_domain_sha256: [u8; 32],
    ) -> Result<Self, ArchiveError> {
        validate_text(file_identity, "journal locator file identity")?;
        if generation == 0
            || record_sequence == 0
            || payload_length == 0
            || payload_offset.checked_add(payload_length).is_none()
        {
            return Err(ArchiveError::InvalidInput("sequence journal page locator"));
        }
        Ok(Self::Journal {
            generation,
            file_identity: file_identity.to_owned(),
            record_sequence,
            payload_offset,
            payload_length,
            page_domain_sha256: hex::encode(page_domain_sha256),
        })
    }

    fn index_pack(
        pack_sha256: [u8; 32],
        payload_offset: u64,
        payload_length: u64,
        page_domain_sha256: [u8; 32],
    ) -> Result<Self, ArchiveError> {
        if payload_length == 0 || payload_offset.checked_add(payload_length).is_none() {
            return Err(ArchiveError::InvalidInput(
                "sequence index-pack page locator",
            ));
        }
        let encoded_pack_hash = hex::encode(pack_sha256);
        Ok(Self::IndexPack {
            pack_relative_path: format!("index-packs/{encoded_pack_hash}.pack"),
            pack_sha256: encoded_pack_hash,
            payload_offset,
            payload_length,
            page_domain_sha256: hex::encode(page_domain_sha256),
        })
    }

    pub(crate) fn index_pack_sha256(&self) -> Result<Option<[u8; 32]>, ArchiveError> {
        match self {
            Self::Journal { .. } => Ok(None),
            Self::IndexPack { pack_sha256, .. } => manifest::parse_hash(pack_sha256).map(Some),
        }
    }

    #[must_use]
    pub(crate) fn index_pack_relative_path(&self) -> Option<&str> {
        match self {
            Self::Journal { .. } => None,
            Self::IndexPack {
                pack_relative_path, ..
            } => Some(pack_relative_path),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceNodeRefV3 {
    chain_id: String,
    source_id: String,
    depth: u8,
    first_local_sequence: u64,
    last_local_sequence: u64,
    row_count: u64,
    logical_manifest_count: u64,
    locator: SequencePageLocatorV3,
}

impl SequenceNodeRefV3 {
    fn from_leaf(page: &SequenceLeafPageV3, locator: SequencePageLocatorV3) -> Self {
        Self {
            chain_id: page.chain_id.clone(),
            source_id: page.source_id.clone(),
            depth: 0,
            first_local_sequence: page.first_local_sequence,
            last_local_sequence: page.last_local_sequence,
            row_count: page.row_count,
            logical_manifest_count: page.logical_manifest_count,
            locator,
        }
    }

    fn from_internal(page: &SequenceInternalPageV3, locator: SequencePageLocatorV3) -> Self {
        Self {
            chain_id: page.chain_id.clone(),
            source_id: page.source_id.clone(),
            depth: page.depth,
            first_local_sequence: page.first_local_sequence,
            last_local_sequence: page.last_local_sequence,
            row_count: page.row_count,
            logical_manifest_count: page.logical_manifest_count,
            locator,
        }
    }

    #[must_use]
    pub const fn depth(&self) -> u8 {
        self.depth
    }

    #[must_use]
    pub const fn first_local_sequence(&self) -> u64 {
        self.first_local_sequence
    }

    #[must_use]
    pub const fn last_local_sequence(&self) -> u64 {
        self.last_local_sequence
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }

    #[must_use]
    pub const fn logical_manifest_count(&self) -> u64 {
        self.logical_manifest_count
    }

    #[must_use]
    pub const fn locator(&self) -> &SequencePageLocatorV3 {
        &self.locator
    }

    pub fn chain_id(&self) -> Result<ChainId, ArchiveError> {
        ChainId::new(self.chain_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("invalid sequence node chain"))
    }

    pub fn source_id(&self) -> Result<SourceId, ArchiveError> {
        SourceId::new(self.source_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("invalid sequence node source"))
    }

    fn journal_commit_evidence(&self) -> Result<(u64, &str, u64, u64, u64, &str), ArchiveError> {
        let SequencePageLocatorV3::Journal {
            generation,
            file_identity,
            record_sequence,
            payload_offset,
            payload_length,
            page_domain_sha256,
        } = &self.locator
        else {
            return Err(ArchiveError::InvalidInput(
                "journal commit root is not journal-located",
            ));
        };
        let payload_end =
            payload_offset
                .checked_add(*payload_length)
                .ok_or(ArchiveError::InvalidInput(
                    "journal root payload end overflows",
                ))?;
        Ok((
            *generation,
            file_identity,
            *record_sequence,
            *payload_offset,
            payload_end,
            page_domain_sha256,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceInternalPageV3 {
    schema: String,
    chain_id: String,
    source_id: String,
    dataset: String,
    depth: u8,
    first_local_sequence: u64,
    last_local_sequence: u64,
    row_count: u64,
    logical_manifest_count: u64,
    children: Vec<SequenceNodeRefV3>,
}

impl SequenceInternalPageV3 {
    pub fn try_new(
        chain_id: ChainId,
        source_id: SourceId,
        depth: u8,
        children: Vec<SequenceNodeRefV3>,
    ) -> Result<Self, ArchiveError> {
        if depth == 0
            || depth > RAW_ARCHIVE_MAXIMUM_SEQUENCE_TREE_DEPTH
            || children.len() < 2
            || children.len() > MAX_SEQUENCE_INTERNAL_CHILDREN
        {
            return Err(ArchiveError::InvalidInput(
                "sequence internal page depth or fanout",
            ));
        }
        let expected_child_depth = depth - 1;
        for child in &children {
            if child.depth != expected_child_depth
                || child.chain_id != chain_id.as_str()
                || child.source_id != source_id.as_str()
            {
                return Err(ArchiveError::InvalidInput(
                    "sequence internal child depth, chain, or source",
                ));
            }
        }
        for pair in children.windows(2) {
            let expected_next =
                pair[0]
                    .last_local_sequence
                    .checked_add(1)
                    .ok_or(ArchiveError::InvalidInput(
                        "sequence internal coverage overflows",
                    ))?;
            if pair[1].first_local_sequence != expected_next {
                return Err(ArchiveError::InvalidInput(
                    "sequence internal children must be exactly contiguous",
                ));
            }
        }
        let first_local_sequence = children[0].first_local_sequence;
        let last_local_sequence = children
            .last()
            .ok_or(ArchiveError::InvalidInput(
                "sequence internal children are empty",
            ))?
            .last_local_sequence;
        let row_count = checked_sum(children.iter().map(|child| child.row_count))?;
        if row_count != sequence_span(first_local_sequence, last_local_sequence)? {
            return Err(ArchiveError::InvalidInput(
                "sequence internal row coverage is not exact",
            ));
        }
        let logical_manifest_count =
            checked_sum(children.iter().map(|child| child.logical_manifest_count))?;
        Ok(Self {
            schema: RAW_SEQUENCE_INTERNAL_SCHEMA_V3.to_owned(),
            chain_id: chain_id.as_str().to_owned(),
            source_id: source_id.as_str().to_owned(),
            dataset: RAW_BYTE_DATASET_V3.to_owned(),
            depth,
            first_local_sequence,
            last_local_sequence,
            row_count,
            logical_manifest_count,
            children,
        })
    }

    #[must_use]
    pub const fn depth(&self) -> u8 {
        self.depth
    }

    #[must_use]
    pub const fn first_local_sequence(&self) -> u64 {
        self.first_local_sequence
    }

    #[must_use]
    pub const fn last_local_sequence(&self) -> u64 {
        self.last_local_sequence
    }

    #[must_use]
    pub fn children(&self) -> &[SequenceNodeRefV3] {
        &self.children
    }

    pub fn chain_id(&self) -> Result<ChainId, ArchiveError> {
        ChainId::new(self.chain_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("invalid sequence internal chain"))
    }

    pub fn source_id(&self) -> Result<SourceId, ArchiveError> {
        SourceId::new(self.source_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("invalid sequence internal source"))
    }
}

#[derive(Debug, Clone)]
pub struct JournalCommitV3 {
    bytes: Vec<u8>,
    prefix: JournalPrefixRefV3,
    root: SequenceNodeRefV3,
}

impl JournalCommitV3 {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub const fn prefix(&self) -> &JournalPrefixRefV3 {
        &self.prefix
    }

    #[must_use]
    pub const fn root(&self) -> &SequenceNodeRefV3 {
        &self.root
    }
}

#[derive(Debug, Clone)]
pub struct JournalGenerationBuilderV3 {
    generation: u64,
    file_identity: String,
    relative_path: String,
    chain_id: Option<String>,
    source_id: Option<String>,
    bytes: Vec<u8>,
    record_count: u64,
}

impl JournalGenerationBuilderV3 {
    pub fn try_new(
        generation: u64,
        file_identity: impl Into<String>,
        relative_path: PathBuf,
    ) -> Result<Self, ArchiveError> {
        let file_identity = file_identity.into();
        validate_text(&file_identity, "journal file identity")?;
        if generation == 0 {
            return Err(ArchiveError::InvalidInput(
                "journal generation must be nonzero",
            ));
        }
        Ok(Self {
            generation,
            file_identity,
            relative_path: checked_relative_string(&relative_path)?,
            chain_id: None,
            source_id: None,
            bytes: Vec::new(),
            record_count: 0,
        })
    }

    pub fn try_resume(
        generation: u64,
        file_identity: impl Into<String>,
        relative_path: PathBuf,
        committed_prefix: Vec<u8>,
        expected: &JournalPrefixRefV3,
        chain_id: &str,
        source_id: &str,
    ) -> Result<Self, ArchiveError> {
        let file_identity = file_identity.into();
        validate_text(&file_identity, "journal file identity")?;
        let relative_path = checked_relative_string(&relative_path)?;
        let prefix_length = u64::try_from(committed_prefix.len())
            .map_err(|_| ArchiveError::InvalidInput("journal prefix exceeds u64"))?;
        if generation == 0
            || generation != expected.generation
            || file_identity != expected.file_identity
            || relative_path != expected.relative_path
            || prefix_length != expected.committed_prefix_length
            || journal_prefix_hash(&committed_prefix)? != expected.committed_prefix_sha256()?
            || count_journal_frames(&committed_prefix)? != expected.committed_record_count
        {
            return Err(ArchiveError::ManifestVerification(
                "journal prefix identity, bytes, or record count do not match the leased root",
            ));
        }
        Ok(Self {
            generation,
            file_identity,
            relative_path,
            chain_id: Some(chain_id.to_owned()),
            source_id: Some(source_id.to_owned()),
            bytes: committed_prefix,
            record_count: expected.committed_record_count,
        })
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn committed_record_count(&self) -> u64 {
        self.record_count
    }

    #[must_use]
    pub fn committed_bytes(&self) -> usize {
        self.bytes.len()
    }

    pub fn push_leaf(
        &mut self,
        page: &SequenceLeafPageV3,
    ) -> Result<SequenceNodeRefV3, ArchiveError> {
        let payload = manifest::canonical_json(page)?;
        let hash = sequence_page_hash(page)?;
        let locator = self.push_frame(&page.chain_id, &page.source_id, 1, 0, &payload, hash)?;
        Ok(SequenceNodeRefV3::from_leaf(page, locator))
    }

    pub fn push_internal(
        &mut self,
        page: &SequenceInternalPageV3,
    ) -> Result<SequenceNodeRefV3, ArchiveError> {
        let payload = manifest::canonical_json(page)?;
        let hash = sequence_internal_page_hash(page)?;
        let locator = self.push_frame(
            &page.chain_id,
            &page.source_id,
            2,
            page.depth,
            &payload,
            hash,
        )?;
        Ok(SequenceNodeRefV3::from_internal(page, locator))
    }

    pub fn commit_prefix(&self, root: &SequenceNodeRefV3) -> Result<JournalCommitV3, ArchiveError> {
        if root.first_local_sequence != 1
            || self.chain_id.as_deref() != Some(root.chain_id.as_str())
            || self.source_id.as_deref() != Some(root.source_id.as_str())
        {
            return Err(ArchiveError::InvalidInput(
                "journal root must cover the source from local sequence one",
            ));
        }
        let (
            generation,
            file_identity,
            root_record_sequence,
            payload_offset,
            payload_end,
            page_hash,
        ) = root.journal_commit_evidence()?;
        let committed_prefix_length = u64::try_from(self.bytes.len())
            .map_err(|_| ArchiveError::InvalidInput("journal prefix exceeds u64"))?;
        if generation != self.generation
            || file_identity != self.file_identity
            || root_record_sequence != self.record_count
            || payload_end > committed_prefix_length
        {
            return Err(ArchiveError::InvalidInput(
                "journal root is outside the committed prefix",
            ));
        }
        self.validate_root_frame(
            root,
            root_record_sequence,
            payload_offset,
            payload_end,
            page_hash,
        )?;
        let committed_prefix_sha256 = journal_prefix_hash(&self.bytes)?;
        Ok(JournalCommitV3 {
            bytes: self.bytes.clone(),
            prefix: JournalPrefixRefV3 {
                generation: self.generation,
                file_identity: self.file_identity.clone(),
                relative_path: self.relative_path.clone(),
                committed_prefix_length,
                committed_record_count: self.record_count,
                committed_prefix_sha256: hex::encode(committed_prefix_sha256),
                root_record_sequence,
                root_first_local_sequence: root.first_local_sequence,
                root_last_local_sequence: root.last_local_sequence,
                root_row_count: root.row_count,
                root_logical_manifest_count: root.logical_manifest_count,
                root_page_domain_sha256: page_hash.to_owned(),
            },
            root: root.clone(),
        })
    }

    fn push_frame(
        &mut self,
        chain_id: &str,
        source_id: &str,
        kind: u8,
        depth: u8,
        payload: &[u8],
        page_domain_sha256: [u8; 32],
    ) -> Result<SequencePageLocatorV3, ArchiveError> {
        if self.record_count >= MAX_JOURNAL_RECORDS {
            return Err(ArchiveError::InvalidInput(
                "journal record count exceeds the active bound",
            ));
        }
        match (&self.chain_id, &self.source_id) {
            (None, None) => {
                self.chain_id = Some(chain_id.to_owned());
                self.source_id = Some(source_id.to_owned());
            }
            (Some(expected_chain), Some(expected_source))
                if expected_chain == chain_id && expected_source == source_id => {}
            _ => {
                return Err(ArchiveError::InvalidInput(
                    "journal records mix chain or source",
                ));
            }
        }
        let record_sequence = self
            .record_count
            .checked_add(1)
            .ok_or(ArchiveError::InvalidInput("journal record count overflows"))?;
        let payload_length = u64::try_from(payload.len())
            .map_err(|_| ArchiveError::InvalidInput("journal payload exceeds u64"))?;
        let frame_start = u64::try_from(self.bytes.len())
            .map_err(|_| ArchiveError::InvalidInput("journal prefix exceeds u64"))?;
        let payload_offset = frame_start.checked_add(JOURNAL_FRAME_HEADER_BYTES).ok_or(
            ArchiveError::InvalidInput("journal payload offset overflows"),
        )?;
        let next_length = payload_offset
            .checked_add(payload_length)
            .ok_or(ArchiveError::InvalidInput("journal frame length overflows"))?;
        if next_length > MAX_JOURNAL_BYTES {
            return Err(ArchiveError::InvalidInput(
                "journal bytes exceed the active bound",
            ));
        }
        self.bytes.extend_from_slice(JOURNAL_FRAME_MAGIC_V3);
        self.bytes.push(kind);
        self.bytes.push(depth);
        self.bytes.extend_from_slice(&[0_u8; 6]);
        self.bytes.extend_from_slice(&record_sequence.to_be_bytes());
        self.bytes.extend_from_slice(&payload_length.to_be_bytes());
        self.bytes.extend_from_slice(&page_domain_sha256);
        if u64::try_from(self.bytes.len()).ok() != Some(payload_offset) {
            return Err(ArchiveError::InvalidInput(
                "journal frame header width changed",
            ));
        }
        self.bytes.extend_from_slice(payload);
        self.record_count = record_sequence;
        SequencePageLocatorV3::journal(
            self.generation,
            &self.file_identity,
            record_sequence,
            payload_offset,
            payload_length,
            page_domain_sha256,
        )
    }

    fn validate_root_frame(
        &self,
        root: &SequenceNodeRefV3,
        record_sequence: u64,
        payload_offset: u64,
        payload_end: u64,
        page_hash: &str,
    ) -> Result<(), ArchiveError> {
        let frame_start = payload_offset
            .checked_sub(JOURNAL_FRAME_HEADER_BYTES)
            .ok_or(ArchiveError::ManifestVerification(
                "journal root frame offset is invalid",
            ))?;
        let frame_start = usize::try_from(frame_start).map_err(|_| {
            ArchiveError::ManifestVerification("journal frame exceeds address space")
        })?;
        let payload_start = usize::try_from(payload_offset).map_err(|_| {
            ArchiveError::ManifestVerification("journal payload exceeds address space")
        })?;
        let payload_end = usize::try_from(payload_end).map_err(|_| {
            ArchiveError::ManifestVerification("journal payload exceeds address space")
        })?;
        let header = self.bytes.get(frame_start..payload_start).ok_or(
            ArchiveError::ManifestVerification("journal root frame is outside the prefix"),
        )?;
        let payload = self.bytes.get(payload_start..payload_end).ok_or(
            ArchiveError::ManifestVerification("journal root payload is outside the prefix"),
        )?;
        let expected_kind = if root.depth == 0 { 1 } else { 2 };
        if header.len() != usize::try_from(JOURNAL_FRAME_HEADER_BYTES).unwrap_or(usize::MAX)
            || header.get(..8) != Some(JOURNAL_FRAME_MAGIC_V3.as_slice())
            || header.get(8) != Some(&expected_kind)
            || header.get(9) != Some(&root.depth)
            || header.get(10..16) != Some([0_u8; 6].as_slice())
        {
            return Err(ArchiveError::ManifestVerification(
                "journal root frame header is invalid",
            ));
        }
        let encoded_record: [u8; 8] = header[16..24]
            .try_into()
            .map_err(|_| ArchiveError::ManifestVerification("journal record number is invalid"))?;
        let encoded_length: [u8; 8] = header[24..32]
            .try_into()
            .map_err(|_| ArchiveError::ManifestVerification("journal payload length is invalid"))?;
        let encoded_hash = header
            .get(32..64)
            .ok_or(ArchiveError::ManifestVerification(
                "journal page hash is missing",
            ))?;
        let expected_hash = manifest::parse_hash(page_hash)?;
        let actual_hash = if root.depth == 0 {
            domain_hash(SEQUENCE_PAGE_HASH_DOMAIN_V3, payload)?
        } else {
            domain_hash(SEQUENCE_INTERNAL_HASH_DOMAIN_V3, payload)?
        };
        if u64::from_be_bytes(encoded_record) != record_sequence
            || u64::from_be_bytes(encoded_length)
                != u64::try_from(payload.len()).unwrap_or(u64::MAX)
            || encoded_hash != expected_hash
            || actual_hash != expected_hash
        {
            return Err(ArchiveError::ManifestVerification(
                "journal root frame does not authenticate the selected page",
            ));
        }
        Ok(())
    }
}

pub(crate) fn append_logical_entry(
    journal: &mut JournalGenerationBuilderV3,
    packs: &IndexPackBytes,
    previous_root: Option<&SequenceNodeRefV3>,
    chain_id: ChainId,
    source_id: SourceId,
    entry: SequenceLeafEntryV3,
) -> Result<SequenceNodeRefV3, ArchiveError> {
    match previous_root {
        None => {
            let page = SequenceLeafPageV3::try_new(chain_id, source_id, vec![entry])?;
            journal.push_leaf(&page)
        }
        Some(node) => match cow_insert(journal, packs, node, &chain_id, &source_id, entry)? {
            CowInsert::Replace(root) => Ok(root),
            CowInsert::Split { left, right } => {
                if left.depth != right.depth {
                    return Err(ArchiveError::InvalidInput(
                        "sequence tree split produced mixed child depths",
                    ));
                }
                let depth = left
                    .depth
                    .checked_add(1)
                    .ok_or(ArchiveError::InvalidInput("sequence tree depth overflows"))?;
                let page =
                    SequenceInternalPageV3::try_new(chain_id, source_id, depth, vec![left, right])?;
                journal.push_internal(&page)
            }
        },
    }
}

enum CowInsert {
    Replace(SequenceNodeRefV3),
    Split {
        left: SequenceNodeRefV3,
        right: SequenceNodeRefV3,
    },
}

fn cow_insert(
    journal: &mut JournalGenerationBuilderV3,
    packs: &IndexPackBytes,
    node: &SequenceNodeRefV3,
    chain_id: &ChainId,
    source_id: &SourceId,
    entry: SequenceLeafEntryV3,
) -> Result<CowInsert, ArchiveError> {
    let expected_next = node
        .last_local_sequence
        .checked_add(1)
        .ok_or(ArchiveError::InvalidInput("local sequence overflows"))?;
    if entry.first_local_sequence != expected_next {
        return Err(ArchiveError::InvalidInput(
            "logical commit does not extend the sequence head",
        ));
    }
    if node.depth == 0 {
        let page = load_sequence_leaf(&journal.bytes, packs, node)?;
        if page.entries.len() < MAX_SEQUENCE_LEAF_ENTRIES {
            let mut entries = page.entries.clone();
            entries.push(entry);
            let new_page =
                SequenceLeafPageV3::try_new(chain_id.clone(), source_id.clone(), entries)?;
            return Ok(CowInsert::Replace(journal.push_leaf(&new_page)?));
        }
        let new_page =
            SequenceLeafPageV3::try_new(chain_id.clone(), source_id.clone(), vec![entry])?;
        let right = journal.push_leaf(&new_page)?;
        return Ok(CowInsert::Split {
            left: node.clone(),
            right,
        });
    }
    let page = load_sequence_internal(&journal.bytes, packs, node)?;
    let last = page
        .children
        .last()
        .ok_or(ArchiveError::ManifestVerification(
            "sequence internal page has no children",
        ))?
        .clone();
    match cow_insert(journal, packs, &last, chain_id, source_id, entry)? {
        CowInsert::Replace(new_last) => {
            let mut children = page.children.clone();
            let last_index =
                children
                    .len()
                    .checked_sub(1)
                    .ok_or(ArchiveError::ManifestVerification(
                        "sequence internal page has no children",
                    ))?;
            children[last_index] = new_last;
            let new_page = SequenceInternalPageV3::try_new(
                chain_id.clone(),
                source_id.clone(),
                page.depth,
                children,
            )?;
            Ok(CowInsert::Replace(journal.push_internal(&new_page)?))
        }
        CowInsert::Split { right, .. } => {
            if page.children.len() >= MAX_SEQUENCE_INTERNAL_CHILDREN {
                let mut all = page.children.clone();
                all.push(right);
                let mid = MAX_SEQUENCE_INTERNAL_CHILDREN / 2;
                let (left_children, right_children) = all.split_at(mid);
                let left_page = SequenceInternalPageV3::try_new(
                    chain_id.clone(),
                    source_id.clone(),
                    page.depth,
                    left_children.to_vec(),
                )?;
                let right_page = SequenceInternalPageV3::try_new(
                    chain_id.clone(),
                    source_id.clone(),
                    page.depth,
                    right_children.to_vec(),
                )?;
                return Ok(CowInsert::Split {
                    left: journal.push_internal(&left_page)?,
                    right: journal.push_internal(&right_page)?,
                });
            }
            let mut children = page.children.clone();
            children.push(right);
            let new_page = SequenceInternalPageV3::try_new(
                chain_id.clone(),
                source_id.clone(),
                page.depth,
                children,
            )?;
            Ok(CowInsert::Replace(journal.push_internal(&new_page)?))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RootBundleV3 {
    schema: String,
    chain_id: String,
    source_id: String,
    dataset: String,
    generation: u64,
    created_at_micros: i64,
    previous_root_sha256: Option<String>,
    journal_prefix: JournalPrefixRefV3,
    sequence_root: SequenceNodeRefV3,
    head_local_sequence: u64,
    logical_manifest_count: u64,
}

impl RootBundleV3 {
    pub fn try_new(
        chain_id: ChainId,
        source_id: SourceId,
        generation: u64,
        previous_root_sha256: Option<[u8; 32]>,
        journal_commit: &JournalCommitV3,
        created_at: KnownTime,
    ) -> Result<Self, ArchiveError> {
        Self::from_prefix_and_root(
            chain_id,
            source_id,
            generation,
            previous_root_sha256,
            &journal_commit.prefix,
            &journal_commit.root,
            created_at,
        )
    }

    fn from_prefix_and_root(
        chain_id: ChainId,
        source_id: SourceId,
        generation: u64,
        previous_root_sha256: Option<[u8; 32]>,
        journal_prefix: &JournalPrefixRefV3,
        sequence_root: &SequenceNodeRefV3,
        created_at: KnownTime,
    ) -> Result<Self, ArchiveError> {
        if generation == 0
            || (generation == 1) != previous_root_sha256.is_none()
            || sequence_root.chain_id != chain_id.as_str()
            || sequence_root.source_id != source_id.as_str()
            || journal_prefix.root_first_local_sequence != 1
            || journal_prefix.root_last_local_sequence != sequence_root.last_local_sequence
            || journal_prefix.root_row_count != sequence_root.row_count
            || journal_prefix.root_logical_manifest_count != sequence_root.logical_manifest_count
        {
            return Err(ArchiveError::InvalidInput(
                "root generation, predecessor, journal, chain, or source",
            ));
        }
        let head_local_sequence = sequence_root.last_local_sequence;
        let logical_manifest_count = sequence_root.logical_manifest_count;
        Ok(Self {
            schema: RAW_ROOT_BUNDLE_SCHEMA_V3.to_owned(),
            chain_id: chain_id.as_str().to_owned(),
            source_id: source_id.as_str().to_owned(),
            dataset: RAW_BYTE_DATASET_V3.to_owned(),
            generation,
            created_at_micros: created_at.unix_micros(),
            previous_root_sha256: previous_root_sha256.map(hex::encode),
            journal_prefix: journal_prefix.clone(),
            sequence_root: sequence_root.clone(),
            head_local_sequence,
            logical_manifest_count,
        })
    }

    #[must_use]
    pub const fn head_local_sequence(&self) -> u64 {
        self.head_local_sequence
    }

    #[must_use]
    pub const fn logical_manifest_count(&self) -> u64 {
        self.logical_manifest_count
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn previous_root_sha256(&self) -> Result<Option<[u8; 32]>, ArchiveError> {
        self.previous_root_sha256
            .as_deref()
            .map(manifest::parse_hash)
            .transpose()
    }

    #[must_use]
    pub const fn journal_prefix(&self) -> &JournalPrefixRefV3 {
        &self.journal_prefix
    }

    #[must_use]
    pub const fn sequence_root(&self) -> &SequenceNodeRefV3 {
        &self.sequence_root
    }

    pub fn chain_id(&self) -> Result<ChainId, ArchiveError> {
        ChainId::new(self.chain_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("invalid root bundle chain"))
    }

    #[must_use]
    pub const fn created_at_micros(&self) -> i64 {
        self.created_at_micros
    }

    pub fn source_id(&self) -> Result<SourceId, ArchiveError> {
        SourceId::new(self.source_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("invalid root bundle source"))
    }
}

pub fn canonical_root_bytes(root: &RootBundleV3) -> Result<Vec<u8>, ArchiveError> {
    manifest::canonical_json(root)
}

pub fn sequence_page_hash(page: &SequenceLeafPageV3) -> Result<[u8; 32], ArchiveError> {
    domain_hash(
        SEQUENCE_PAGE_HASH_DOMAIN_V3,
        &manifest::canonical_json(page)?,
    )
}

pub fn sequence_internal_page_hash(
    page: &SequenceInternalPageV3,
) -> Result<[u8; 32], ArchiveError> {
    domain_hash(
        SEQUENCE_INTERNAL_HASH_DOMAIN_V3,
        &manifest::canonical_json(page)?,
    )
}

fn page_domain_hash(
    kind: IndexPackPageKindV3,
    page_bytes: &[u8],
) -> Result<[u8; 32], ArchiveError> {
    let domain = match kind {
        IndexPackPageKindV3::SequenceLeaf => SEQUENCE_PAGE_HASH_DOMAIN_V3,
        IndexPackPageKindV3::SequenceInternal => SEQUENCE_INTERNAL_HASH_DOMAIN_V3,
        IndexPackPageKindV3::ReceiptHint => RECEIPT_HINT_HASH_DOMAIN_V3,
    };
    domain_hash(domain, page_bytes)
}

pub fn root_bundle_hash(root: &RootBundleV3) -> Result<[u8; 32], ArchiveError> {
    domain_hash(ROOT_BUNDLE_HASH_DOMAIN_V3, &canonical_root_bytes(root)?)
}

pub fn journal_prefix_hash(committed_prefix: &[u8]) -> Result<[u8; 32], ArchiveError> {
    if committed_prefix.is_empty() {
        return Err(ArchiveError::InvalidInput(
            "journal committed prefix must be nonempty",
        ));
    }
    domain_hash(JOURNAL_PREFIX_HASH_DOMAIN_V3, committed_prefix)
}

pub(crate) fn journal_payload_bytes<'a>(
    prefix: &'a [u8],
    locator: &SequencePageLocatorV3,
) -> Result<&'a [u8], ArchiveError> {
    let SequencePageLocatorV3::Journal {
        payload_offset,
        payload_length,
        page_domain_sha256,
        ..
    } = locator
    else {
        return Err(ArchiveError::ManifestVerification(
            "sequence page is not journal-located",
        ));
    };
    let start = usize::try_from(*payload_offset)
        .map_err(|_| ArchiveError::ManifestVerification("journal payload exceeds address space"))?;
    let end = usize::try_from(payload_offset.checked_add(*payload_length).ok_or(
        ArchiveError::ManifestVerification("journal payload end overflows"),
    )?)
    .map_err(|_| ArchiveError::ManifestVerification("journal payload exceeds address space"))?;
    let payload = prefix
        .get(start..end)
        .ok_or(ArchiveError::ManifestVerification(
            "journal payload is outside the committed prefix",
        ))?;
    authenticate_sequence_payload(payload, page_domain_sha256)?;
    Ok(payload)
}

fn index_pack_payload_bytes<'a>(
    packs: &'a IndexPackBytes,
    locator: &SequencePageLocatorV3,
) -> Result<&'a [u8], ArchiveError> {
    let SequencePageLocatorV3::IndexPack {
        pack_sha256,
        payload_offset,
        payload_length,
        page_domain_sha256,
        pack_relative_path,
    } = locator
    else {
        return Err(ArchiveError::ManifestVerification(
            "sequence page is not index-pack-located",
        ));
    };
    let hash = manifest::parse_hash(pack_sha256)?;
    let object = packs.get(&hash).ok_or(ArchiveError::ManifestVerification(
        "sequence index pack is not loaded",
    ))?;
    if hex::encode(manifest::sha256(object)) != *pack_sha256 {
        return Err(ArchiveError::ManifestVerification(
            "loaded index pack bytes do not match the locator hash",
        ));
    }
    let expected_path = format!("index-packs/{}.pack", hex::encode(hash));
    if pack_relative_path != &expected_path {
        return Err(ArchiveError::ManifestVerification(
            "index pack locator path is not content-addressed",
        ));
    }
    let start = usize::try_from(*payload_offset).map_err(|_| {
        ArchiveError::ManifestVerification("index pack payload exceeds address space")
    })?;
    let end = usize::try_from(payload_offset.checked_add(*payload_length).ok_or(
        ArchiveError::ManifestVerification("index pack payload end overflows"),
    )?)
    .map_err(|_| ArchiveError::ManifestVerification("index pack payload exceeds address space"))?;
    let payload = object
        .get(start..end)
        .ok_or(ArchiveError::ManifestVerification(
            "index pack payload is outside the object",
        ))?;
    authenticate_sequence_payload(payload, page_domain_sha256)?;
    Ok(payload)
}

fn authenticate_sequence_payload(
    payload: &[u8],
    page_domain_sha256: &str,
) -> Result<(), ArchiveError> {
    let expected = manifest::parse_hash(page_domain_sha256)?;
    let actual = if parse_sequence_leaf_page(payload).is_ok() {
        domain_hash(SEQUENCE_PAGE_HASH_DOMAIN_V3, payload)?
    } else if parse_sequence_internal_page(payload).is_ok() {
        domain_hash(SEQUENCE_INTERNAL_HASH_DOMAIN_V3, payload)?
    } else {
        return Err(ArchiveError::ManifestVerification(
            "sequence payload is not a valid sequence page",
        ));
    };
    if actual != expected {
        return Err(ArchiveError::ManifestVerification(
            "sequence payload hash does not authenticate the page",
        ));
    }
    Ok(())
}

fn sequence_page_payload<'a>(
    journal: &'a [u8],
    packs: &'a IndexPackBytes,
    locator: &SequencePageLocatorV3,
) -> Result<&'a [u8], ArchiveError> {
    match locator {
        SequencePageLocatorV3::Journal { .. } => journal_payload_bytes(journal, locator),
        SequencePageLocatorV3::IndexPack { .. } => index_pack_payload_bytes(packs, locator),
    }
}

pub(crate) fn load_sequence_leaf(
    journal: &[u8],
    packs: &IndexPackBytes,
    node: &SequenceNodeRefV3,
) -> Result<SequenceLeafPageV3, ArchiveError> {
    if node.depth != 0 {
        return Err(ArchiveError::ManifestVerification(
            "sequence node is not a leaf",
        ));
    }
    let payload = sequence_page_payload(journal, packs, &node.locator)?;
    let page = parse_sequence_leaf_page(payload)?;
    if page.first_local_sequence != node.first_local_sequence
        || page.last_local_sequence != node.last_local_sequence
        || page.row_count != node.row_count
        || page.logical_manifest_count != node.logical_manifest_count
        || page.chain_id != node.chain_id
        || page.source_id != node.source_id
    {
        return Err(ArchiveError::ManifestVerification(
            "sequence leaf page does not match the sequence node",
        ));
    }
    Ok(page)
}

pub(crate) fn load_sequence_internal(
    journal: &[u8],
    packs: &IndexPackBytes,
    node: &SequenceNodeRefV3,
) -> Result<SequenceInternalPageV3, ArchiveError> {
    if node.depth == 0 {
        return Err(ArchiveError::ManifestVerification(
            "sequence node is not internal",
        ));
    }
    let payload = sequence_page_payload(journal, packs, &node.locator)?;
    let page = parse_sequence_internal_page(payload)?;
    if page.depth != node.depth
        || page.first_local_sequence != node.first_local_sequence
        || page.last_local_sequence != node.last_local_sequence
        || page.row_count != node.row_count
        || page.logical_manifest_count != node.logical_manifest_count
        || page.chain_id != node.chain_id
        || page.source_id != node.source_id
    {
        return Err(ArchiveError::ManifestVerification(
            "sequence internal page does not match the sequence node",
        ));
    }
    Ok(page)
}

pub(crate) fn journal_needs_rotation(record_count: u64, committed_bytes: u64, depth: u8) -> bool {
    let spine = u64::from(depth).saturating_add(1);
    let page_budget = spine.saturating_mul(
        storage_ports::RAW_ARCHIVE_MAXIMUM_SEQUENCE_PAGE_BYTES
            .saturating_add(JOURNAL_FRAME_HEADER_BYTES),
    );
    record_count.saturating_add(spine) > MAX_JOURNAL_RECORDS
        || committed_bytes.saturating_add(page_budget) > MAX_JOURNAL_BYTES
}

pub(crate) fn pack_journal_leaves(
    chain_id: ChainId,
    source_id: SourceId,
    pack_generation: u64,
    root: &SequenceNodeRefV3,
    journal: &[u8],
    packs: &IndexPackBytes,
    hint_pages: &[ReceiptHintPageV3],
) -> Result<(BuiltIndexPackV3, BTreeMap<u64, SequenceNodeRefV3>), ArchiveError> {
    let mut builder = IndexPackBuilderV3::try_new(chain_id, source_id, pack_generation)?;
    let mut packed_pages = BTreeMap::new();
    collect_journal_leaves(root, journal, packs, &mut builder, &mut packed_pages)?;
    if packed_pages.is_empty() {
        return Err(ArchiveError::InvalidInput(
            "index packing requires at least one journal-located leaf",
        ));
    }
    for page in hint_pages {
        builder.push_receipt_hint(page)?;
    }
    let pack = builder.finish()?;
    let mut packed_refs = BTreeMap::new();
    for (first_sequence, (index, page)) in packed_pages {
        packed_refs.insert(first_sequence, pack.sequence_leaf_ref(index, &page)?);
    }
    Ok((pack, packed_refs))
}

fn collect_journal_leaves(
    node: &SequenceNodeRefV3,
    journal: &[u8],
    packs: &IndexPackBytes,
    builder: &mut IndexPackBuilderV3,
    packed_pages: &mut BTreeMap<u64, (usize, SequenceLeafPageV3)>,
) -> Result<(), ArchiveError> {
    if node.depth == 0 {
        if matches!(node.locator, SequencePageLocatorV3::Journal { .. }) {
            let page = load_sequence_leaf(journal, packs, node)?;
            let index = builder.push_sequence_leaf(&page)?;
            packed_pages.insert(page.first_local_sequence, (index, page));
        }
        return Ok(());
    }
    let page = load_sequence_internal(journal, packs, node)?;
    for child in page.children() {
        collect_journal_leaves(child, journal, packs, builder, packed_pages)?;
    }
    Ok(())
}

pub(crate) fn rewrite_tree_into_journal(
    journal: &mut JournalGenerationBuilderV3,
    packs: &IndexPackBytes,
    packed_leaves: &BTreeMap<u64, SequenceNodeRefV3>,
    previous_journal: &[u8],
    node: &SequenceNodeRefV3,
) -> Result<SequenceNodeRefV3, ArchiveError> {
    if node.depth == 0 {
        if let Some(packed) = packed_leaves.get(&node.first_local_sequence) {
            if packed.last_local_sequence != node.last_local_sequence {
                return Err(ArchiveError::ManifestVerification(
                    "packed leaf coverage does not match the sequence node",
                ));
            }
            return Ok(packed.clone());
        }
        if matches!(node.locator, SequencePageLocatorV3::IndexPack { .. }) {
            return Ok(node.clone());
        }
        return Err(ArchiveError::ManifestVerification(
            "journal leaf was not included in the index pack",
        ));
    }
    let page = load_sequence_internal(previous_journal, packs, node)?;
    let mut children = Vec::with_capacity(page.children.len());
    for child in page.children() {
        children.push(rewrite_tree_into_journal(
            journal,
            packs,
            packed_leaves,
            previous_journal,
            child,
        )?);
    }
    if children == page.children {
        if matches!(node.locator, SequencePageLocatorV3::Journal { .. }) {
            let rewritten = SequenceInternalPageV3::try_new(
                page.chain_id()?,
                page.source_id()?,
                page.depth,
                children,
            )?;
            return journal.push_internal(&rewritten);
        }
        return Ok(node.clone());
    }
    let rewritten =
        SequenceInternalPageV3::try_new(page.chain_id()?, page.source_id()?, page.depth, children)?;
    journal.push_internal(&rewritten)
}

pub(crate) fn seed_rotated_journal_root(
    journal: &mut JournalGenerationBuilderV3,
    packs: &IndexPackBytes,
    packed_leaves: &BTreeMap<u64, SequenceNodeRefV3>,
    previous_journal: &[u8],
    node: &SequenceNodeRefV3,
) -> Result<SequenceNodeRefV3, ArchiveError> {
    if node.depth == 0 {
        let page = load_sequence_leaf(previous_journal, packs, node)?;
        return journal.push_leaf(&page);
    }
    rewrite_tree_into_journal(journal, packs, packed_leaves, previous_journal, node)
}

pub(crate) fn collect_leaf_entries(
    node: &SequenceNodeRefV3,
    journal: &[u8],
    packs: &IndexPackBytes,
) -> Result<Vec<SequenceLeafEntryV3>, ArchiveError> {
    let mut entries = Vec::new();
    collect_leaf_entries_walk(node, journal, packs, &mut entries)?;
    Ok(entries)
}

fn collect_leaf_entries_walk(
    node: &SequenceNodeRefV3,
    journal: &[u8],
    packs: &IndexPackBytes,
    output: &mut Vec<SequenceLeafEntryV3>,
) -> Result<(), ArchiveError> {
    if node.depth == 0 {
        let page = load_sequence_leaf(journal, packs, node)?;
        output.extend(page.entries.iter().cloned());
        return Ok(());
    }
    let page = load_sequence_internal(journal, packs, node)?;
    for child in page.children() {
        collect_leaf_entries_walk(child, journal, packs, output)?;
    }
    Ok(())
}

pub(crate) fn replace_range_with_packed_entry(
    journal: &mut JournalGenerationBuilderV3,
    packs: &IndexPackBytes,
    previous_root: &SequenceNodeRefV3,
    packed: SequenceLeafEntryV3,
) -> Result<SequenceNodeRefV3, ArchiveError> {
    let previous_bytes = journal.bytes.clone();
    let entries = collect_leaf_entries(previous_root, &previous_bytes, packs)?;
    let chain_id = previous_root.chain_id()?;
    let source_id = previous_root.source_id()?;
    let replaced = splice_packed_entry(entries, packed)?;
    let mut root = None;
    for entry in replaced {
        root = Some(append_logical_entry(
            journal,
            packs,
            root.as_ref(),
            chain_id.clone(),
            source_id.clone(),
            entry,
        )?);
    }
    root.ok_or(ArchiveError::InvalidInput(
        "packed sequence rewrite produced an empty tree",
    ))
}

fn splice_packed_entry(
    entries: Vec<SequenceLeafEntryV3>,
    packed: SequenceLeafEntryV3,
) -> Result<Vec<SequenceLeafEntryV3>, ArchiveError> {
    let first = packed.first_local_sequence;
    let last = packed.last_local_sequence;
    let mut replaced = Vec::new();
    let mut index = 0_usize;
    let mut inserted = false;
    while index < entries.len() {
        let entry = &entries[index];
        if entry.last_local_sequence < first || entry.first_local_sequence > last {
            replaced.push(entry.clone());
            index += 1;
            continue;
        }
        if entry.first_local_sequence < first || entry.last_local_sequence > last {
            return Err(ArchiveError::InvalidInput(
                "packed range must replace whole leaf entries",
            ));
        }
        if inserted {
            return Err(ArchiveError::InvalidInput(
                "packed range overlaps multiple disjoint leaf spans",
            ));
        }
        let mut covered_first = None;
        let mut covered_last = None;
        while index < entries.len()
            && entries[index].first_local_sequence >= first
            && entries[index].last_local_sequence <= last
        {
            if matches!(entries[index].storage, SequenceStorageRefV3::Packed { .. }) {
                return Err(ArchiveError::InvalidInput(
                    "packed range must select uncompacted logical leaves",
                ));
            }
            if covered_first.is_none() {
                covered_first = Some(entries[index].first_local_sequence);
            }
            covered_last = Some(entries[index].last_local_sequence);
            index += 1;
        }
        if covered_first != Some(first) || covered_last != Some(last) {
            return Err(ArchiveError::InvalidInput(
                "packed range must cover an exact contiguous logical span",
            ));
        }
        replaced.push(packed.clone());
        inserted = true;
    }
    if !inserted {
        return Err(ArchiveError::InvalidInput(
            "packed range does not match any sequence leaf entries",
        ));
    }
    Ok(replaced)
}

fn count_journal_frames(bytes: &[u8]) -> Result<u64, ArchiveError> {
    let header_len = usize::try_from(JOURNAL_FRAME_HEADER_BYTES).map_err(|_| {
        ArchiveError::ManifestVerification("journal frame header exceeds address space")
    })?;
    let mut offset = 0_usize;
    let mut count = 0_u64;
    while offset < bytes.len() {
        let header_end =
            offset
                .checked_add(header_len)
                .ok_or(ArchiveError::ManifestVerification(
                    "journal frame header overflows",
                ))?;
        let header = bytes
            .get(offset..header_end)
            .ok_or(ArchiveError::ManifestVerification(
                "journal frame header is truncated",
            ))?;
        if header.len() != header_len || header.get(..8) != Some(JOURNAL_FRAME_MAGIC_V3.as_slice())
        {
            return Err(ArchiveError::ManifestVerification(
                "journal frame magic is invalid",
            ));
        }
        let encoded_record: [u8; 8] = header[16..24]
            .try_into()
            .map_err(|_| ArchiveError::ManifestVerification("journal record number is invalid"))?;
        let encoded_length: [u8; 8] = header[24..32]
            .try_into()
            .map_err(|_| ArchiveError::ManifestVerification("journal payload length is invalid"))?;
        let payload_length = usize::try_from(u64::from_be_bytes(encoded_length)).map_err(|_| {
            ArchiveError::ManifestVerification("journal payload exceeds address space")
        })?;
        let frame_end =
            header_end
                .checked_add(payload_length)
                .ok_or(ArchiveError::ManifestVerification(
                    "journal frame length overflows",
                ))?;
        if frame_end > bytes.len() {
            return Err(ArchiveError::ManifestVerification(
                "journal frame payload is truncated",
            ));
        }
        count = count
            .checked_add(1)
            .ok_or(ArchiveError::ManifestVerification(
                "journal record count overflows",
            ))?;
        if u64::from_be_bytes(encoded_record) != count {
            return Err(ArchiveError::ManifestVerification(
                "journal record numbers are not contiguous",
            ));
        }
        offset = frame_end;
    }
    if offset != bytes.len() {
        return Err(ArchiveError::ManifestVerification(
            "journal prefix has trailing unframed bytes",
        ));
    }
    Ok(count)
}

fn combined_pack_hash(inputs: &[PackedLogicalInputV3]) -> Result<[u8; 32], ArchiveError> {
    let capacity = inputs
        .len()
        .checked_mul(80)
        .and_then(|bytes| bytes.checked_add(8))
        .ok_or(ArchiveError::InvalidInput(
            "raw pack combined hash input capacity overflows",
        ))?;
    let mut evidence = Vec::with_capacity(capacity);
    evidence.extend_from_slice(
        &u64::try_from(inputs.len())
            .map_err(|_| ArchiveError::InvalidInput("raw pack input count exceeds u64"))?
            .to_be_bytes(),
    );
    for input in inputs {
        evidence.extend_from_slice(&manifest::parse_hash(&input.manifest_sha256)?);
        evidence.extend_from_slice(&manifest::parse_hash(&input.rolling_content_sha256)?);
        evidence.extend_from_slice(&input.first_local_sequence.to_be_bytes());
        evidence.extend_from_slice(&input.last_local_sequence.to_be_bytes());
    }
    domain_hash(PACK_COMBINED_HASH_DOMAIN_V3, &evidence)
}

pub(crate) fn domain_hash(domain: &[u8], bytes: &[u8]) -> Result<[u8; 32], ArchiveError> {
    let domain_length = u64::try_from(domain.len())
        .map_err(|_| ArchiveError::InvalidInput("archive hash domain exceeds u64"))?;
    let bytes_length = u64::try_from(bytes.len())
        .map_err(|_| ArchiveError::InvalidInput("archive hash input exceeds u64"))?;
    let mut digest = Sha256::new();
    digest.update(domain_length.to_be_bytes());
    digest.update(domain);
    digest.update(bytes_length.to_be_bytes());
    digest.update(bytes);
    Ok(digest.finalize().into())
}

fn checked_relative_string(path: &Path) -> Result<String, ArchiveError> {
    fs::validate_relative(path)?;
    let value = path.to_str().ok_or(ArchiveError::UnsafePath)?;
    if value.len() > RAW_ARCHIVE_MAXIMUM_RELATIVE_PATH_BYTES {
        return Err(ArchiveError::UnsafePath);
    }
    Ok(value.to_owned())
}

fn validate_partition(partition: &str) -> Result<(), ArchiveError> {
    validate_text(partition, "raw archive partition")?;
    fs::validate_relative(Path::new(partition))
}

fn validate_text(value: &str, name: &'static str) -> Result<(), ArchiveError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > MAX_TEXT_FIELD_BYTES
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(ArchiveError::InvalidInput(name));
    }
    Ok(())
}

fn sequence_span(first: u64, last: u64) -> Result<u64, ArchiveError> {
    if first == 0 || last < first {
        return Err(ArchiveError::InvalidInput("local sequence range"));
    }
    last.checked_sub(first)
        .and_then(|span| span.checked_add(1))
        .ok_or(ArchiveError::InvalidInput("local sequence range overflows"))
}

fn checked_sum(mut values: impl Iterator<Item = u64>) -> Result<u64, ArchiveError> {
    values.try_fold(0_u64, |sum, value| {
        sum.checked_add(value)
            .ok_or(ArchiveError::InvalidInput("sequence page count overflows"))
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use domain_types::{ChainId, KnownTime, SourceId};

    use super::{
        IndexPackBuilderV3, IndexPackBytes, JournalCommitV3, JournalGenerationBuilderV3,
        LogicalCommitDescriptorV3, LogicalCommitManifestV3, LogicalObjectDescriptorV3,
        MAX_JOURNAL_RECORDS, MAX_SEQUENCE_INTERNAL_CHILDREN, PackedLogicalInputV3,
        PackedObjectDescriptorV3, RAW_LOGICAL_COMMIT_SCHEMA_V3, RAW_SEQUENCE_LEAF_SCHEMA_V3,
        RawPackManifestV3, ReceiptHintEntryV3, ReceiptHintPageV3, RootBundleV3,
        SequenceInternalPageV3, SequenceLeafEntryV3, SequenceLeafPageV3, SequencePageLocatorV3,
        append_logical_entry, canonical_root_bytes, journal_needs_rotation, journal_prefix_hash,
        load_sequence_leaf, logical_commit_domain_hash, pack_journal_leaves,
        parse_logical_commit_manifest, parse_pack_manifest, parse_receipt_hint_page,
        parse_root_bundle, parse_sequence_internal_page, parse_sequence_leaf_page,
        replace_range_with_packed_entry, root_bundle_hash, seed_rotated_journal_root,
        sequence_internal_page_hash, sequence_page_hash,
    };

    fn packed(first: u64, last: u64, marker: u8, logical_count: u64) -> SequenceLeafEntryV3 {
        SequenceLeafEntryV3::try_new_packed(
            first,
            last,
            format!("packs/pack-{marker}.json"),
            [marker; 32],
            (last - first + 1) * 100,
            last - first + 1,
            logical_count,
            "date=2026-08-03/hour=12",
        )
        .unwrap()
    }

    fn logical(first: u64, last: u64, marker: u8) -> SequenceLeafEntryV3 {
        SequenceLeafEntryV3::try_new_logical(
            first,
            last,
            format!("manifests/logical-{marker}.json"),
            [marker; 32],
            (last - first + 1) * 100,
            last - first + 1,
            "date=2026-08-03/hour=12",
        )
        .unwrap()
    }

    fn logical_page(first: u64, last: u64, marker: u8) -> SequenceLeafPageV3 {
        SequenceLeafPageV3::try_new(
            ChainId::new("mainnet").unwrap(),
            SourceId::new("node-fills").unwrap(),
            vec![logical(first, last, marker)],
        )
        .unwrap()
    }

    fn journal_commit(page: &SequenceLeafPageV3) -> JournalCommitV3 {
        let mut journal = JournalGenerationBuilderV3::try_new(
            7,
            "journal-identity-7",
            PathBuf::from("journals/generation-7.log"),
        )
        .unwrap();
        let root = journal.push_leaf(page).unwrap();
        journal.commit_prefix(&root).unwrap()
    }

    #[test]
    fn journal_prefix_is_nonempty_bounded_and_immutable_by_contract() {
        assert!(
            JournalGenerationBuilderV3::try_new(0, "journal", PathBuf::from("journals/zero.log"),)
                .is_err()
        );
        assert!(
            JournalGenerationBuilderV3::try_new(1, "journal", PathBuf::from("../escape.log"),)
                .is_err()
        );
        let page = SequenceLeafPageV3::try_new(
            ChainId::new("mainnet").unwrap(),
            SourceId::new("node-fills").unwrap(),
            vec![logical(1, 1, 1)],
        )
        .unwrap();
        let commit = journal_commit(&page);
        assert!(!commit.bytes().is_empty());
        assert_eq!(commit.prefix().committed_record_count, 1);
        assert_eq!(commit.prefix().root_last_local_sequence, 1);
    }

    #[test]
    fn sequence_leaf_requires_exact_contiguous_coverage() {
        let chain = ChainId::new("mainnet").unwrap();
        let source = SourceId::new("node-fills").unwrap();
        let page = SequenceLeafPageV3::try_new(
            chain.clone(),
            source.clone(),
            vec![logical(1, 2, 1), logical(3, 4, 2)],
        )
        .unwrap();
        assert_eq!(page.first_local_sequence(), 1);
        assert_eq!(page.last_local_sequence(), 4);
        assert_eq!(page.row_count(), 4);

        assert!(
            SequenceLeafPageV3::try_new(
                chain.clone(),
                source.clone(),
                vec![logical(1, 2, 1), logical(4, 5, 2)],
            )
            .is_err()
        );
        assert!(
            SequenceLeafPageV3::try_new(chain, source, vec![logical(1, 3, 1), logical(3, 4, 2)],)
                .is_err()
        );
    }

    #[test]
    fn sequence_leaf_wire_decode_revalidates_invariants_and_canonical_bytes() {
        let page = SequenceLeafPageV3::try_new(
            ChainId::new("mainnet").unwrap(),
            SourceId::new("node-fills").unwrap(),
            vec![logical(1, 2, 1), logical(3, 4, 2)],
        )
        .unwrap();
        let bytes = super::manifest::canonical_json(&page).unwrap();
        assert_eq!(parse_sequence_leaf_page(&bytes).unwrap(), page);

        let text = String::from_utf8(bytes).unwrap();
        let gap = text.replacen(
            r#""first_local_sequence":3"#,
            r#""first_local_sequence":4"#,
            1,
        );
        assert!(parse_sequence_leaf_page(gap.as_bytes()).is_err());
        let wrong_schema = text.replacen(
            RAW_SEQUENCE_LEAF_SCHEMA_V3,
            "hyperliquid-alpha-desk/archive-raw-sequence-leaf/v999",
            1,
        );
        assert!(parse_sequence_leaf_page(wrong_schema.as_bytes()).is_err());
        let noncanonical = format!(" {text}");
        assert!(parse_sequence_leaf_page(noncanonical.as_bytes()).is_err());
    }

    #[test]
    fn root_generation_previous_hash_and_head_are_atomic() {
        let chain = ChainId::new("mainnet").unwrap();
        let source = SourceId::new("node-fills").unwrap();
        let page = SequenceLeafPageV3::try_new(
            chain.clone(),
            source.clone(),
            vec![logical(1, 2, 1), logical(3, 4, 2)],
        )
        .unwrap();
        let created = KnownTime::from_unix_micros(1_000).unwrap();
        let commit = journal_commit(&page);

        assert!(
            RootBundleV3::try_new(
                chain.clone(),
                source.clone(),
                1,
                Some([9; 32]),
                &commit,
                created,
            )
            .is_err()
        );
        assert!(
            RootBundleV3::try_new(chain.clone(), source.clone(), 2, None, &commit, created,)
                .is_err()
        );

        let root = RootBundleV3::try_new(chain, source, 1, None, &commit, created).unwrap();
        assert_eq!(root.head_local_sequence(), 4);
        assert_eq!(root.logical_manifest_count(), 2);
    }

    #[test]
    fn root_canonical_json_is_frozen() {
        let chain = ChainId::new("mainnet").unwrap();
        let source = SourceId::new("node-fills").unwrap();
        let page = SequenceLeafPageV3::try_new(
            chain.clone(),
            source.clone(),
            vec![logical(1, 2, 1), logical(3, 4, 2)],
        )
        .unwrap();
        let commit = journal_commit(&page);
        let root = RootBundleV3::try_new(
            chain,
            source,
            1,
            None,
            &commit,
            KnownTime::from_unix_micros(1_000).unwrap(),
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(canonical_root_bytes(&root).unwrap()).unwrap(),
            r#"{"schema":"hyperliquid-alpha-desk/archive-raw-root-bundle/v3","chain_id":"mainnet","source_id":"node-fills","dataset":"raw_source_observations_byte_v3","generation":1,"created_at_micros":1000,"previous_root_sha256":null,"journal_prefix":{"generation":7,"file_identity":"journal-identity-7","relative_path":"journals/generation-7.log","committed_prefix_length":960,"committed_record_count":1,"committed_prefix_sha256":"4d6de9c99c68fda4f3ebba325bde0a65744fd34c6bd7675198f7f77cd3c3ece1","root_record_sequence":1,"root_first_local_sequence":1,"root_last_local_sequence":4,"root_row_count":4,"root_logical_manifest_count":2,"root_page_domain_sha256":"28f6e13d48c75f7a02835a3c56979e88f5cbf0a0de4b4c68f38a7b76dce35fa8"},"sequence_root":{"chain_id":"mainnet","source_id":"node-fills","depth":0,"first_local_sequence":1,"last_local_sequence":4,"row_count":4,"logical_manifest_count":2,"locator":{"kind":"journal","generation":7,"file_identity":"journal-identity-7","record_sequence":1,"payload_offset":64,"payload_length":896,"page_domain_sha256":"28f6e13d48c75f7a02835a3c56979e88f5cbf0a0de4b4c68f38a7b76dce35fa8"}},"head_local_sequence":4,"logical_manifest_count":2}"#
        );
        assert_eq!(
            hex::encode(sequence_page_hash(&page).unwrap()),
            "28f6e13d48c75f7a02835a3c56979e88f5cbf0a0de4b4c68f38a7b76dce35fa8"
        );
        assert_eq!(
            hex::encode(root_bundle_hash(&root).unwrap()),
            "a9dffa95f760140c960a3f75532d45cc593e7344556cac89db97de3e850593c1"
        );
    }

    #[test]
    fn packed_leaf_preserves_exact_sequence_and_logical_counts() {
        let packed = SequenceLeafEntryV3::try_new_packed(
            1,
            8,
            "packs/pack-1.json",
            [0x44; 32],
            2_048,
            8,
            3,
            "date=2026-08-03/hour=12",
        )
        .unwrap();
        assert_eq!(packed.logical_manifest_count(), 3);
        assert!(
            SequenceLeafEntryV3::try_new_packed(
                1,
                8,
                "packs/pack-1.json",
                [0x44; 32],
                2_048,
                7,
                3,
                "date=2026-08-03/hour=12",
            )
            .is_err()
        );
        assert!(
            SequenceLeafEntryV3::try_new_packed(
                1,
                8,
                "packs/pack-1.json",
                [0x44; 32],
                2_048,
                8,
                1,
                "date=2026-08-03/hour=12",
            )
            .is_err()
        );
        assert!(
            SequenceLeafEntryV3::try_new_logical(
                1,
                1,
                "a".repeat(storage_ports::RAW_ARCHIVE_MAXIMUM_RELATIVE_PATH_BYTES + 1),
                [1; 32],
                1,
                1,
                "date=2026-08-03/hour=12",
            )
            .is_err()
        );
    }

    #[test]
    fn receipt_hint_keys_are_sorted_unique_and_non_authoritative() {
        let chain = ChainId::new("mainnet").unwrap();
        let source = SourceId::new("node-fills").unwrap();
        let first = ReceiptHintEntryV3::try_new([1; 32], 1, 2).unwrap();
        let second = ReceiptHintEntryV3::try_new([2; 32], 3, 4).unwrap();
        let page = ReceiptHintPageV3::try_new(
            chain.clone(),
            source.clone(),
            vec![first.clone(), second.clone()],
        )
        .unwrap();
        assert_eq!(page.candidate_range([2; 32]), Some((3, 4)));
        assert_eq!(page.candidate_range([3; 32]), None);
        assert!(
            ReceiptHintPageV3::try_new(chain.clone(), source.clone(), vec![second, first.clone()])
                .is_err()
        );
        assert!(ReceiptHintPageV3::try_new(chain, source, vec![first.clone(), first]).is_err());
    }

    #[test]
    fn index_pack_layout_and_authentication_are_derived_from_exact_bytes() {
        let chain = ChainId::new("mainnet").unwrap();
        let source = SourceId::new("node-fills").unwrap();
        let leaf =
            SequenceLeafPageV3::try_new(chain.clone(), source.clone(), vec![logical(1, 2, 1)])
                .unwrap();
        let hints = ReceiptHintPageV3::try_new(
            chain.clone(),
            source.clone(),
            vec![ReceiptHintEntryV3::try_new([1; 32], 1, 2).unwrap()],
        )
        .unwrap();
        let mut builder = IndexPackBuilderV3::try_new(chain, source, 1).unwrap();
        let leaf_index = builder.push_sequence_leaf(&leaf).unwrap();
        builder.push_receipt_hint(&hints).unwrap();
        let pack = builder.finish().unwrap();

        assert_eq!(pack.manifest().page_count(), 2);
        assert_eq!(
            pack.manifest().object_relative_path,
            format!("index-packs/{}.pack", hex::encode(pack.object_sha256()))
        );
        assert!(pack.verify_bytes(pack.bytes()).is_ok());
        let leaf_ref = pack.sequence_leaf_ref(leaf_index, &leaf).unwrap();
        assert_eq!(leaf_ref.depth(), 0);

        let mut substituted = pack.bytes().to_vec();
        substituted[0] ^= 1;
        assert!(pack.verify_bytes(&substituted).is_err());
        assert!(
            pack.sequence_leaf_ref(leaf_index, &logical_page(3, 4, 2))
                .is_err()
        );
    }

    #[test]
    fn old_journal_prefix_remains_exact_after_later_append() {
        let chain = ChainId::new("mainnet").unwrap();
        let source = SourceId::new("node-fills").unwrap();
        let first_page =
            SequenceLeafPageV3::try_new(chain.clone(), source.clone(), vec![logical(1, 2, 1)])
                .unwrap();
        let second_page =
            SequenceLeafPageV3::try_new(chain.clone(), source.clone(), vec![logical(3, 4, 2)])
                .unwrap();
        let mut journal = JournalGenerationBuilderV3::try_new(
            7,
            "journal-identity-7",
            PathBuf::from("journals/generation-7.log"),
        )
        .unwrap();
        let first = journal.push_leaf(&first_page).unwrap();
        let old = journal.commit_prefix(&first).unwrap();
        let second = journal.push_leaf(&second_page).unwrap();
        let internal =
            SequenceInternalPageV3::try_new(chain, source, 1, vec![first, second]).unwrap();
        let root = journal.push_internal(&internal).unwrap();
        let extended = journal.commit_prefix(&root).unwrap();

        assert!(extended.bytes().starts_with(old.bytes()));
        assert_eq!(
            old.prefix().committed_prefix_sha256,
            hex::encode(journal_prefix_hash(old.bytes()).unwrap())
        );
        assert_ne!(
            old.prefix().committed_prefix_sha256,
            extended.prefix().committed_prefix_sha256
        );
        let mut substituted = old.bytes().to_vec();
        substituted[0] ^= 1;
        assert_ne!(
            old.prefix().committed_prefix_sha256,
            hex::encode(journal_prefix_hash(&substituted).unwrap())
        );
    }

    #[test]
    fn journal_commit_rejects_cross_builder_root_substitution() {
        let chain = ChainId::new("mainnet").unwrap();
        let source = SourceId::new("node-fills").unwrap();
        let page_a =
            SequenceLeafPageV3::try_new(chain.clone(), source.clone(), vec![logical(1, 2, 1)])
                .unwrap();
        let page_b = SequenceLeafPageV3::try_new(chain, source, vec![logical(1, 2, 2)]).unwrap();
        let mut builder_a = JournalGenerationBuilderV3::try_new(
            7,
            "journal-identity-a",
            PathBuf::from("journals/generation-7-a.log"),
        )
        .unwrap();
        builder_a.push_leaf(&page_a).unwrap();
        let mut builder_b = JournalGenerationBuilderV3::try_new(
            7,
            "journal-identity-b",
            PathBuf::from("journals/generation-7-b.log"),
        )
        .unwrap();
        let root_b = builder_b.push_leaf(&page_b).unwrap();
        assert!(builder_a.commit_prefix(&root_b).is_err());

        let mut same_identity = JournalGenerationBuilderV3::try_new(
            7,
            "journal-identity-a",
            PathBuf::from("journals/generation-7-a.log"),
        )
        .unwrap();
        let same_identity_root_b = same_identity.push_leaf(&page_b).unwrap();
        assert!(builder_a.commit_prefix(&same_identity_root_b).is_err());
    }

    fn packed_input(marker: u8, first: u64, last: u64, row_start: u64) -> PackedLogicalInputV3 {
        let object_hash = hex::encode([marker; 32]);
        let rolling_hash = hex::encode([marker.wrapping_add(1); 32]);
        let schema_hash = hex::encode(crate::schema::raw_schema_fingerprint().unwrap());
        let start_offset = u64::from(marker) * 100;
        let end_offset = start_offset + 99;
        let object_path = format!(
            "chain=mainnet/dataset=raw_source_observations_byte_v2/source=node-fills/date=1970-01-01/hour=00/objects/epoch=epoch-{marker}/sequences={first}-{last}/offsets={start_offset}-{end_offset}/part-{object_hash}.parquet"
        );
        let canonical = format!(
            r#"{{"schema":"hyperliquid-alpha-desk/archive-raw-batch-manifest/v2","producer_build_id":"build-{marker}","created_at_micros":42,"batch":{{"chain_id":"mainnet","source_id":"node-fills","source_version":"capture-v1","observation_class":"auxiliary-ledger","cursor_policy":"monotonic-byte-offset","cursor_epoch":"epoch-{marker}","start_offset":{start_offset},"end_offset":{end_offset},"first_local_sequence":{first},"last_local_sequence":{last},"first_received_wall_micros":100,"last_received_wall_micros":200,"parser_schema_version":"raw-parser-v1","spool_manifest_blake3":"{}","spool_segment_blake3":"{}","rolling_content_sha256":"{rolling_hash}"}},"object":{{"relative_path":"{object_path}","sha256":"{object_hash}","size_bytes":512,"row_count":{},"schema_fingerprint_sha256":"{schema_hash}"}}}}"#,
            hex::encode([marker.wrapping_add(2); 32]),
            hex::encode([marker.wrapping_add(3); 32]),
            last - first + 1,
        )
        .into_bytes();
        let hash = super::manifest::sha256(&canonical);
        PackedLogicalInputV3::try_new_v2(canonical, hash, row_start).unwrap()
    }

    #[test]
    fn raw_pack_manifest_preserves_exact_originals_and_row_slices() {
        let object = PackedObjectDescriptorV3::try_new(
            PathBuf::from(format!(
                "date=1970-01-01/hour=00/packs/pack-{}.parquet",
                hex::encode([0x77; 32])
            )),
            [0x77; 32],
            8_192,
            4,
        )
        .unwrap();
        assert_eq!(
            object.schema_fingerprint_sha256,
            hex::encode(crate::schema::raw_schema_fingerprint().unwrap())
        );
        let inputs = vec![packed_input(1, 1, 2, 0), packed_input(2, 3, 4, 2)];
        let pack = RawPackManifestV3::try_new(
            inputs.clone(),
            object.clone(),
            KnownTime::from_unix_micros(9_000).unwrap(),
        )
        .unwrap();
        assert_eq!(pack.first_local_sequence(), 1);
        assert_eq!(pack.last_local_sequence(), 4);
        assert_eq!(pack.logical_manifest_count(), 2);

        let overlapping_rows = vec![packed_input(1, 1, 2, 0), packed_input(2, 3, 4, 1)];
        assert!(
            RawPackManifestV3::try_new(
                overlapping_rows,
                object.clone(),
                KnownTime::from_unix_micros(9_000).unwrap(),
            )
            .is_err()
        );
        assert!(
            RawPackManifestV3::try_new(
                vec![inputs[0].clone(), inputs[0].clone()],
                object,
                KnownTime::from_unix_micros(9_000).unwrap(),
            )
            .is_err()
        );
    }

    #[test]
    fn packed_input_rejects_manifest_byte_or_hash_substitution() {
        let bytes = br#"{"schema":"logical-v3"}"#.to_vec();
        assert!(PackedLogicalInputV3::try_new_v2(bytes, [0xAA; 32], 0).is_err());

        let valid = packed_input(1, 1, 1, 0);
        let substituted = valid
            .canonical_manifest_json
            .replacen("chain=mainnet/", "chain=wrong/", 1)
            .into_bytes();
        let substituted_hash = super::manifest::sha256(&substituted);
        assert!(PackedLogicalInputV3::try_new_v2(substituted, substituted_hash, 0).is_err());

        let valid = packed_input(2, 2, 2, 0);
        for invalid in [
            valid.canonical_manifest_json.replacen(
                r#""first_received_wall_micros":100"#,
                r#""first_received_wall_micros":-1"#,
                1,
            ),
            valid.canonical_manifest_json.replacen(
                r#""producer_build_id":"build-2""#,
                r#""producer_build_id":" build-2""#,
                1,
            ),
            valid
                .canonical_manifest_json
                .replacen("capture-v1", r"capture-\u0085-v1", 1),
        ] {
            let invalid = invalid.into_bytes();
            let invalid_hash = super::manifest::sha256(&invalid);
            assert!(PackedLogicalInputV3::try_new_v2(invalid, invalid_hash, 0).is_err());
        }
    }

    #[test]
    fn internal_sequence_page_requires_exact_child_coverage_and_depth() {
        let chain = ChainId::new("mainnet").unwrap();
        let source = SourceId::new("node-fills").unwrap();
        let first_page =
            SequenceLeafPageV3::try_new(chain.clone(), source.clone(), vec![logical(1, 2, 1)])
                .unwrap();
        let second_page =
            SequenceLeafPageV3::try_new(chain.clone(), source.clone(), vec![logical(3, 4, 2)])
                .unwrap();
        let mut journal = JournalGenerationBuilderV3::try_new(
            7,
            "journal-identity-7",
            PathBuf::from("journals/generation-7.log"),
        )
        .unwrap();
        let first = journal.push_leaf(&first_page).unwrap();
        let second = journal.push_leaf(&second_page).unwrap();
        let internal = SequenceInternalPageV3::try_new(
            chain.clone(),
            source.clone(),
            1,
            vec![first.clone(), second.clone()],
        )
        .unwrap();
        assert_eq!(internal.first_local_sequence(), 1);
        assert_eq!(internal.last_local_sequence(), 4);
        assert_eq!(internal.depth(), 1);
        let root_ref = journal.push_internal(&internal).unwrap();
        assert_eq!(root_ref.depth(), 1);
        let mut index_pack = IndexPackBuilderV3::try_new(chain.clone(), source.clone(), 9).unwrap();
        let internal_index = index_pack.push_sequence_internal(&internal).unwrap();
        let index_pack = index_pack.finish().unwrap();
        let packed_root = index_pack
            .sequence_internal_ref(internal_index, &internal)
            .unwrap();
        assert_eq!(packed_root.depth(), 1);
        let commit = journal.commit_prefix(&root_ref).unwrap();
        assert_eq!(commit.prefix().committed_record_count, 3);

        assert!(
            SequenceInternalPageV3::try_new(
                chain.clone(),
                source.clone(),
                2,
                vec![first.clone(), second.clone()],
            )
            .is_err()
        );
        let gap_page =
            SequenceLeafPageV3::try_new(chain.clone(), source.clone(), vec![logical(5, 6, 3)])
                .unwrap();
        let gap = journal.push_leaf(&gap_page).unwrap();
        assert!(SequenceInternalPageV3::try_new(chain, source, 1, vec![first, gap]).is_err());
    }

    fn frozen_logical_commit() -> LogicalCommitManifestV3 {
        let object_sha256 = [0x44; 32];
        let descriptor = LogicalCommitDescriptorV3::try_new(
            ChainId::new("mainnet").unwrap(),
            SourceId::new("node-fills").unwrap(),
            "capture-v1",
            "auxiliary-ledger",
            "epoch-1",
            10,
            11,
            1,
            2,
            1_000,
            1_000,
            "raw-parser-v1",
            [0x11; 32],
            [0x22; 32],
            [0x33; 32],
        )
        .unwrap();
        let relative = super::logical_object_relative_path(&descriptor, object_sha256).unwrap();
        let object =
            LogicalObjectDescriptorV3::try_new(PathBuf::from(relative), object_sha256, 512, 2)
                .unwrap();
        LogicalCommitManifestV3::try_new(
            "build-v3",
            KnownTime::from_unix_micros(1_000).unwrap(),
            descriptor,
            object,
        )
        .unwrap()
    }

    #[test]
    fn logical_commit_canonical_json_and_domain_hash_are_frozen() {
        let commit = frozen_logical_commit();
        let bytes = super::manifest::canonical_json(&commit).unwrap();
        assert_eq!(parse_logical_commit_manifest(&bytes).unwrap(), commit);
        assert!(
            bytes.starts_with(
                br#"{"schema":"hyperliquid-alpha-desk/archive-raw-logical-commit/v3""#
            )
        );
        assert_eq!(
            hex::encode(super::manifest::sha256(&bytes)),
            "59f22dcb99185b21ce203d8ebc4fc4ee33654a99683b97e0c8403c0d3a9b6472"
        );
        assert_eq!(
            hex::encode(logical_commit_domain_hash(&commit).unwrap()),
            "2607aac9756fa3f3caca4b868919b99160296d98501ff80f4a16a690599bdd36"
        );
        let mutated = String::from_utf8(bytes.clone()).unwrap().replacen(
            RAW_LOGICAL_COMMIT_SCHEMA_V3,
            "hyperliquid-alpha-desk/archive-raw-logical-commit/v999",
            1,
        );
        assert!(parse_logical_commit_manifest(mutated.as_bytes()).is_err());
        assert!(
            parse_logical_commit_manifest(
                format!(" {0}", String::from_utf8(bytes).unwrap()).as_bytes()
            )
            .is_err()
        );
    }

    #[test]
    fn remaining_wire_decoders_revalidate_canonical_bytes() {
        let chain = ChainId::new("mainnet").unwrap();
        let source = SourceId::new("node-fills").unwrap();
        let first_page =
            SequenceLeafPageV3::try_new(chain.clone(), source.clone(), vec![logical(1, 2, 1)])
                .unwrap();
        let second_page =
            SequenceLeafPageV3::try_new(chain.clone(), source.clone(), vec![logical(3, 4, 2)])
                .unwrap();
        let mut journal = JournalGenerationBuilderV3::try_new(
            7,
            "journal-identity-7",
            PathBuf::from("journals/generation-7.log"),
        )
        .unwrap();
        let first = journal.push_leaf(&first_page).unwrap();
        let second = journal.push_leaf(&second_page).unwrap();
        let internal =
            SequenceInternalPageV3::try_new(chain.clone(), source.clone(), 1, vec![first, second])
                .unwrap();
        let internal_bytes = super::manifest::canonical_json(&internal).unwrap();
        assert_eq!(
            parse_sequence_internal_page(&internal_bytes).unwrap(),
            internal
        );
        assert_eq!(
            hex::encode(sequence_internal_page_hash(&internal).unwrap()),
            "069af0994969844ed07a71356ed7b5ec044b16a1e8d6b4639380767a1a17a7b5"
        );
        let mutated = String::from_utf8(internal_bytes.clone()).unwrap().replacen(
            r#""depth":1"#,
            r#""depth":2"#,
            1,
        );
        assert!(parse_sequence_internal_page(mutated.as_bytes()).is_err());

        let hints = ReceiptHintPageV3::try_new(
            chain.clone(),
            source.clone(),
            vec![ReceiptHintEntryV3::try_new([1; 32], 1, 2).unwrap()],
        )
        .unwrap();
        let hint_bytes = super::manifest::canonical_json(&hints).unwrap();
        assert_eq!(parse_receipt_hint_page(&hint_bytes).unwrap(), hints);
        let authoritative = String::from_utf8(hint_bytes).unwrap().replacen(
            r#""authoritative":false"#,
            r#""authoritative":true"#,
            1,
        );
        assert!(parse_receipt_hint_page(authoritative.as_bytes()).is_err());

        let commit = journal_commit(&first_page);
        let root = RootBundleV3::try_new(
            chain,
            source,
            1,
            None,
            &commit,
            KnownTime::from_unix_micros(1_000).unwrap(),
        )
        .unwrap();
        let root_bytes = canonical_root_bytes(&root).unwrap();
        assert_eq!(parse_root_bundle(&root_bytes).unwrap(), root);
        let mutated_root = String::from_utf8(root_bytes).unwrap().replacen(
            r#""generation":1"#,
            r#""generation":2"#,
            1,
        );
        assert!(parse_root_bundle(mutated_root.as_bytes()).is_err());
    }

    #[test]
    fn pack_manifest_decoder_accepts_typed_v2_and_v3_inputs() {
        let object = PackedObjectDescriptorV3::try_new(
            PathBuf::from(format!(
                "date=1970-01-01/hour=00/packs/pack-{}.parquet",
                hex::encode([0x77; 32])
            )),
            [0x77; 32],
            8_192,
            4,
        )
        .unwrap();
        let inputs = vec![packed_input(1, 1, 2, 0), packed_input(2, 3, 4, 2)];
        let pack =
            RawPackManifestV3::try_new(inputs, object, KnownTime::from_unix_micros(9_000).unwrap())
                .unwrap();
        let bytes = super::manifest::canonical_json(&pack).unwrap();
        assert_eq!(parse_pack_manifest(&bytes).unwrap(), pack);

        let v3 = frozen_logical_commit();
        let v3_bytes = super::manifest::canonical_json(&v3).unwrap();
        let v3_hash = super::manifest::sha256(&v3_bytes);
        let packed_v3 = PackedLogicalInputV3::try_new_v3(v3_bytes.clone(), v3_hash, 0).unwrap();
        assert_eq!(packed_v3.first_local_sequence, 1);
        assert_eq!(packed_v3.last_local_sequence, 2);
        assert_eq!(packed_v3.original_schema, "raw-v3");
        assert!(PackedLogicalInputV3::try_new_v3(v3_bytes.clone(), [0; 32], 0).is_err());
        let mutated = format!(
            " {0}",
            String::from_utf8(v3_bytes).expect("logical commit JSON is UTF-8")
        );
        assert!(PackedLogicalInputV3::try_new_v3(mutated.into_bytes(), v3_hash, 0).is_err());
    }

    #[test]
    fn journal_resume_and_tree_append_keep_old_prefixes_exact() {
        let chain = ChainId::new("mainnet").unwrap();
        let source = SourceId::new("node-fills").unwrap();
        let mut journal = JournalGenerationBuilderV3::try_new(
            1,
            super::journal_file_identity(1).unwrap(),
            PathBuf::from("journals/generation-1.log"),
        )
        .unwrap();
        let first = append_logical_entry(
            &mut journal,
            &IndexPackBytes::new(),
            None,
            chain.clone(),
            source.clone(),
            logical(1, 1, 1),
        )
        .unwrap();
        let first_commit = journal.commit_prefix(&first).unwrap();
        let resumed = JournalGenerationBuilderV3::try_resume(
            1,
            super::journal_file_identity(1).unwrap(),
            PathBuf::from("journals/generation-1.log"),
            first_commit.bytes().to_vec(),
            first_commit.prefix(),
            chain.as_str(),
            source.as_str(),
        )
        .unwrap();
        assert_eq!(resumed.record_count, 1);
        let mut substituted = first_commit.bytes().to_vec();
        substituted[0] ^= 1;
        assert!(
            JournalGenerationBuilderV3::try_resume(
                1,
                super::journal_file_identity(1).unwrap(),
                PathBuf::from("journals/generation-1.log"),
                substituted,
                first_commit.prefix(),
                chain.as_str(),
                source.as_str(),
            )
            .is_err()
        );

        let second = append_logical_entry(
            &mut journal,
            &IndexPackBytes::new(),
            Some(&first),
            chain,
            source,
            logical(2, 2, 2),
        )
        .unwrap();
        assert_eq!(second.first_local_sequence(), 1);
        assert_eq!(second.last_local_sequence(), 2);
        let extended = journal.commit_prefix(&second).unwrap();
        assert!(extended.bytes().starts_with(first_commit.bytes()));
    }

    #[test]
    fn journal_rotation_packs_leaves_and_keeps_old_prefix_exact() {
        let chain = ChainId::new("mainnet").unwrap();
        let source = SourceId::new("node-fills").unwrap();
        let empty = IndexPackBytes::new();
        let mut journal = JournalGenerationBuilderV3::try_new(
            1,
            super::journal_file_identity(1).unwrap(),
            PathBuf::from("journals/generation-1.log"),
        )
        .unwrap();
        let mut root = append_logical_entry(
            &mut journal,
            &empty,
            None,
            chain.clone(),
            source.clone(),
            logical(1, 1, 1),
        )
        .unwrap();
        for sequence in 2..=257 {
            let marker = u8::try_from((sequence % 250) + 1).unwrap();
            root = append_logical_entry(
                &mut journal,
                &empty,
                Some(&root),
                chain.clone(),
                source.clone(),
                logical(sequence, sequence, marker),
            )
            .unwrap();
        }
        assert_eq!(root.depth(), 1);
        let old = journal.commit_prefix(&root).unwrap();
        let (pack, packed_leaves) = pack_journal_leaves(
            chain.clone(),
            source.clone(),
            1,
            &root,
            old.bytes(),
            &empty,
            &[],
        )
        .unwrap();
        assert_eq!(packed_leaves.len(), 2);
        let mut packs = IndexPackBytes::new();
        packs.insert(pack.object_sha256(), pack.bytes().to_vec());
        let mut rotated = JournalGenerationBuilderV3::try_new(
            2,
            super::journal_file_identity(2).unwrap(),
            PathBuf::from("journals/generation-2.log"),
        )
        .unwrap();
        let rotated_root =
            seed_rotated_journal_root(&mut rotated, &packs, &packed_leaves, old.bytes(), &root)
                .unwrap();
        assert_eq!(rotated.committed_record_count(), 1);
        assert!(matches!(
            rotated_root.locator(),
            SequencePageLocatorV3::Journal { generation: 2, .. }
        ));
        let page = super::load_sequence_internal(old.bytes(), &packs, &rotated_root);
        assert!(page.is_err());
        let internals =
            super::load_sequence_internal(rotated.bytes.as_slice(), &packs, &rotated_root).unwrap();
        assert_eq!(internals.children().len(), 2);
        for child in internals.children() {
            assert!(matches!(
                child.locator(),
                SequencePageLocatorV3::IndexPack { .. }
            ));
            load_sequence_leaf(&[], &packs, child).unwrap();
        }
        let mut mutated = pack.bytes().to_vec();
        mutated[0] ^= 1;
        let mut bad = IndexPackBytes::new();
        bad.insert(pack.object_sha256(), mutated);
        assert!(load_sequence_leaf(&[], &bad, internals.children().first().unwrap()).is_err());
        assert!(old.bytes().starts_with(&old.bytes()[..8]));
        let resumed = JournalGenerationBuilderV3::try_resume(
            1,
            super::journal_file_identity(1).unwrap(),
            PathBuf::from("journals/generation-1.log"),
            old.bytes().to_vec(),
            old.prefix(),
            chain.as_str(),
            source.as_str(),
        )
        .unwrap();
        assert_eq!(
            resumed.committed_record_count(),
            old.prefix().committed_record_count()
        );
    }

    #[test]
    fn full_internal_page_splits_instead_of_failing_closed_on_pack() {
        let chain = ChainId::new("mainnet").unwrap();
        let source = SourceId::new("node-fills").unwrap();
        let empty = IndexPackBytes::new();
        let mut journal = JournalGenerationBuilderV3::try_new(
            1,
            super::journal_file_identity(1).unwrap(),
            PathBuf::from("journals/generation-1.log"),
        )
        .unwrap();
        let mut children = Vec::new();
        for sequence in 1..MAX_SEQUENCE_INTERNAL_CHILDREN {
            let sequence = u64::try_from(sequence).unwrap();
            let page = SequenceLeafPageV3::try_new(
                chain.clone(),
                source.clone(),
                vec![logical(sequence, sequence, 1)],
            )
            .unwrap();
            children.push(journal.push_leaf(&page).unwrap());
        }
        let leaf_span = u64::try_from(super::MAX_SEQUENCE_LEAF_ENTRIES).unwrap();
        let last_first = u64::try_from(MAX_SEQUENCE_INTERNAL_CHILDREN).unwrap();
        let last_end = last_first
            .checked_add(leaf_span)
            .unwrap()
            .checked_sub(1)
            .unwrap();
        let last_entries = (last_first..=last_end)
            .map(|sequence| logical(sequence, sequence, 2))
            .collect();
        let last_page =
            SequenceLeafPageV3::try_new(chain.clone(), source.clone(), last_entries).unwrap();
        children.push(journal.push_leaf(&last_page).unwrap());
        let internal =
            SequenceInternalPageV3::try_new(chain.clone(), source.clone(), 1, children).unwrap();
        let root = journal.push_internal(&internal).unwrap();
        assert_eq!(root.depth(), 1);
        let next = last_end.checked_add(1).unwrap();
        let split_root = append_logical_entry(
            &mut journal,
            &empty,
            Some(&root),
            chain,
            source,
            logical(next, next, 3),
        )
        .unwrap();
        assert_eq!(split_root.depth(), 2);
        assert_eq!(split_root.last_local_sequence(), next);
    }

    #[test]
    fn journal_rotation_triggers_before_record_or_byte_limits() {
        assert!(!journal_needs_rotation(1, 128, 0));
        assert!(journal_needs_rotation(MAX_JOURNAL_RECORDS, 128, 0));
        assert!(journal_needs_rotation(1, super::MAX_JOURNAL_BYTES, 0));
    }

    #[test]
    fn packed_leaf_replaces_exact_logical_span_and_keeps_old_prefix() {
        let chain = ChainId::new("mainnet").unwrap();
        let source = SourceId::new("node-fills").unwrap();
        let empty = IndexPackBytes::new();
        let mut journal = JournalGenerationBuilderV3::try_new(
            1,
            super::journal_file_identity(1).unwrap(),
            PathBuf::from("journals/generation-1.log"),
        )
        .unwrap();
        let mut root = append_logical_entry(
            &mut journal,
            &empty,
            None,
            chain.clone(),
            source.clone(),
            logical(1, 1, 1),
        )
        .unwrap();
        root = append_logical_entry(
            &mut journal,
            &empty,
            Some(&root),
            chain.clone(),
            source.clone(),
            logical(2, 2, 2),
        )
        .unwrap();
        root = append_logical_entry(
            &mut journal,
            &empty,
            Some(&root),
            chain.clone(),
            source.clone(),
            logical(3, 3, 3),
        )
        .unwrap();
        let old = journal.commit_prefix(&root).unwrap();
        let packed_root =
            replace_range_with_packed_entry(&mut journal, &empty, &root, packed(1, 2, 9, 2))
                .unwrap();
        let rewritten = journal.commit_prefix(&packed_root).unwrap();
        assert!(rewritten.bytes().starts_with(old.bytes()));
        let page = load_sequence_leaf(rewritten.bytes(), &empty, &packed_root).unwrap();
        assert_eq!(page.entries().len(), 2);
        assert!(matches!(
            page.entries()[0].storage(),
            super::SequenceStorageRefV3::Packed { .. }
        ));
        assert_eq!(page.entries()[0].first_local_sequence(), 1);
        assert_eq!(page.entries()[0].last_local_sequence(), 2);
        assert!(matches!(
            page.entries()[1].storage(),
            super::SequenceStorageRefV3::Logical { .. }
        ));
        assert_eq!(page.entries()[1].first_local_sequence(), 3);
        assert!(
            replace_range_with_packed_entry(
                &mut journal,
                &empty,
                &packed_root,
                packed(1, 2, 9, 2),
            )
            .is_err()
        );
    }
}
