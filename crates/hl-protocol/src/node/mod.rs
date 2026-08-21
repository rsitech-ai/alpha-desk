//! Versioned, minimal Hyperliquid node-output envelopes.
//!
//! These parsers classify source evidence and retain its exact bytes. Canonical
//! domain mapping belongs to the canonicalizer, not to this boundary crate.

pub mod misc;
pub mod order_status;
pub mod qualification;
pub mod raw_book_diff;
pub mod state_snapshot;
pub mod trade;
pub mod transaction;
pub mod v1;
