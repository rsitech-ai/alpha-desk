//! Official public WebSocket envelopes and a data-driven subscription registry.
//!
//! Transport and session lifecycle belong to capture. These parsers classify
//! source evidence and retain exact bytes. They never mark observations committed.
//!
//! ponytail: payload bodies stay JSON. T05 owns `/info` shapes such as
//! `ClearinghouseState`, `OpenOrders`, and `SpotState` when those types exist.

mod message;
mod registry;
mod snapshot;
mod subscription;
mod user_events;

pub use message::{
    WsAck, WsHeartbeat, WsIncremental, WsObservation, WsSnapshot, WsUnknown, parse_ws_message,
};
pub use registry::{
    PayloadShape, SnapshotPolicy, VariantClassifier, WsFamily, families, family_by_channel,
    family_by_identifier,
};
pub use snapshot::{SnapshotRelation, relate_snapshots};
pub use subscription::{
    WsAckMethod, WsSubscription, encode_subscribe, encode_unsubscribe, parse_subscription,
};
pub use user_events::UserEventKind;
