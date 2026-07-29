use domain_types::{BlockHeight, ChainId};

const GAP_INCIDENT_CONTEXT: &str = "hyperliquid-alpha-desk/committed-gap/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GapRange {
    start: BlockHeight,
    end_inclusive: BlockHeight,
}

pub(super) fn gap_incident_id(chain_id: &ChainId, gap: GapRange) -> String {
    let mut hasher = blake3::Hasher::new_derive_key(GAP_INCIDENT_CONTEXT);
    let chain_bytes = chain_id.as_str().as_bytes();
    let chain_length = match u64::try_from(chain_bytes.len()) {
        Ok(length) => length,
        Err(_) => unreachable!("chain identifiers cannot exceed u64 framing"),
    };
    hasher.update(&chain_length.to_be_bytes());
    hasher.update(chain_bytes);
    hasher.update(&gap.start().get().to_be_bytes());
    hasher.update(&gap.end_inclusive().get().to_be_bytes());
    format!("inc_{}", hasher.finalize().to_hex())
}

impl GapRange {
    pub(super) const fn new(start: BlockHeight, end_inclusive: BlockHeight) -> Self {
        Self {
            start,
            end_inclusive,
        }
    }

    #[must_use]
    pub const fn start(self) -> BlockHeight {
        self.start
    }

    #[must_use]
    pub const fn end_inclusive(self) -> BlockHeight {
        self.end_inclusive
    }
}
