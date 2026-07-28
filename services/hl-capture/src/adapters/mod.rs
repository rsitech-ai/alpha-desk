//! Source-specific I/O adapters.
//!
//! Implementations belong behind the `hl_protocol` source ports. Vendor payload
//! types must not cross this boundary into canonical domain crates.

mod node_files;
mod node_stream;

pub use node_files::{NodeBlockDirectoryConfig, NodeBlockDirectorySource};
pub use node_stream::{
    NodeFileConfig, NodeLineFileSource, NodeQuarantineRecord, NodeReceiveClock, SystemNodeClock,
};
