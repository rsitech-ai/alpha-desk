use domain_types::{BlockHeight, BlockRange, ChainId, ManifestId};

const REPLAY_RECEIPT_SCHEMA: &[u8] = b"hyperliquid-alpha-desk/replay-receipt/v1";
const REPLAY_RECEIPT_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/replay-receipt-hash/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayStatus {
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplayManifestIdentity {
    pub(crate) manifest_id: ManifestId,
    pub(crate) manifest_sha256: [u8; 32],
    pub(crate) block_range: BlockRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayReceipt {
    status: ReplayStatus,
    chain_id: ChainId,
    planned_range: BlockRange,
    start_state_hash: [u8; 32],
    final_state_hash: [u8; 32],
    reducer_set_version: String,
    applied_block_count: u64,
    last_applied_height: Option<BlockHeight>,
    last_canonical_block_hash: Option<[u8; 32]>,
    manifests: Vec<ReplayManifestIdentity>,
}

impl ReplayReceipt {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        status: ReplayStatus,
        chain_id: ChainId,
        planned_range: BlockRange,
        start_state_hash: [u8; 32],
        final_state_hash: [u8; 32],
        reducer_set_version: String,
        applied_block_count: u64,
        last_applied_height: Option<BlockHeight>,
        last_canonical_block_hash: Option<[u8; 32]>,
        manifests: Vec<ReplayManifestIdentity>,
    ) -> Self {
        Self {
            status,
            chain_id,
            planned_range,
            start_state_hash,
            final_state_hash,
            reducer_set_version,
            applied_block_count,
            last_applied_height,
            last_canonical_block_hash,
            manifests,
        }
    }

    #[must_use]
    pub const fn status(&self) -> ReplayStatus {
        self.status
    }

    #[must_use]
    pub const fn applied_block_count(&self) -> u64 {
        self.applied_block_count
    }

    #[must_use]
    pub const fn last_applied_height(&self) -> Option<BlockHeight> {
        self.last_applied_height
    }

    #[must_use]
    pub const fn final_state_hash(&self) -> [u8; 32] {
        self.final_state_hash
    }

    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        frame(&mut bytes, REPLAY_RECEIPT_SCHEMA);
        bytes.push(match self.status {
            ReplayStatus::Completed => 1,
            ReplayStatus::Cancelled => 2,
        });
        frame(&mut bytes, self.chain_id.as_str().as_bytes());
        bytes.extend_from_slice(&self.planned_range.start_inclusive.get().to_be_bytes());
        bytes.extend_from_slice(&self.planned_range.end_inclusive.get().to_be_bytes());
        bytes.extend_from_slice(&self.start_state_hash);
        bytes.extend_from_slice(&self.final_state_hash);
        frame(&mut bytes, self.reducer_set_version.as_bytes());
        bytes.extend_from_slice(&self.applied_block_count.to_be_bytes());
        match (self.last_applied_height, self.last_canonical_block_hash) {
            (Some(height), Some(hash)) => {
                bytes.push(1);
                bytes.extend_from_slice(&height.get().to_be_bytes());
                bytes.extend_from_slice(&hash);
            }
            (None, None) => bytes.push(0),
            _ => unreachable!("replay receipt last-block fields are constructed together"),
        }
        extend_count(&mut bytes, self.manifests.len());
        for manifest in &self.manifests {
            frame(&mut bytes, manifest.manifest_id.as_str().as_bytes());
            bytes.extend_from_slice(&manifest.manifest_sha256);
            bytes.extend_from_slice(&manifest.block_range.start_inclusive.get().to_be_bytes());
            bytes.extend_from_slice(&manifest.block_range.end_inclusive.get().to_be_bytes());
        }
        bytes
    }

    #[must_use]
    pub fn receipt_hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new_derive_key(REPLAY_RECEIPT_HASH_CONTEXT);
        hasher.update(&self.canonical_bytes());
        *hasher.finalize().as_bytes()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayOutcome {
    Completed(ReplayReceipt),
    Cancelled(ReplayReceipt),
}

fn extend_count(bytes: &mut Vec<u8>, count: usize) {
    let count = u64::try_from(count).expect("replay manifest count cannot exceed u64");
    bytes.extend_from_slice(&count.to_be_bytes());
}

fn frame(bytes: &mut Vec<u8>, value: &[u8]) {
    let length = u64::try_from(value.len()).expect("replay receipt field cannot exceed u64");
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(value);
}
