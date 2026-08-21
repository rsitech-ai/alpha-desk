//! Source-specific I/O adapters.
//!
//! Implementations belong behind the `hl_protocol` source ports. Vendor payload
//! types must not cross this boundary into canonical domain crates.

mod node_files;
mod node_stream;

pub mod evm_local;
pub mod evm_rpc;
pub mod evm_s3;
pub mod historical_s3;
pub mod info_rest;
pub mod providers;
pub mod public_ws;

pub use node_files::{
    NodeBlockDirectoryConfig, NodeBlockDirectorySource, NodeSnapshotDirectoryConfig,
    NodeSnapshotDirectorySource,
};
pub use node_stream::{
    NodeFileConfig, NodeLineFileSource, NodeLineTailState, NodeQuarantineRecord, NodeReceiveClock,
    SystemNodeClock,
};
