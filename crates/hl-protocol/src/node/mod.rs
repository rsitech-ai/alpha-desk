//! Versioned, minimal Hyperliquid node-output envelopes.
//!
//! These parsers classify source evidence and retain its exact bytes. Canonical
//! domain mapping belongs to the canonicalizer, not to this boundary crate.

pub mod qualification;
pub mod v1;
