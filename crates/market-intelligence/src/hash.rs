pub fn digest(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"alpha-desk/market-intelligence/v1");
    for part in parts {
        hasher.update(&(u64::try_from(part.len()).unwrap_or(u64::MAX).to_le_bytes()));
        hasher.update(part);
    }
    *hasher.finalize().as_bytes()
}

pub fn require_non_empty(value: &str, field: &'static str) -> Result<(), crate::MarketError> {
    if value.is_empty() || value.trim() != value {
        Err(crate::MarketError::EmptyIdentifier { field })
    } else {
        Ok(())
    }
}
