use crate::ObservationClass;

/// How a family turns `data` into snapshot vs incremental observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotPolicy {
    FullReplace,
    Tagged,
    Incremental,
}

/// Expected JSON shape of the `data` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadShape {
    Object,
    Array,
    Either,
}

/// Extra fail-closed classification beyond payload shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantClassifier {
    None,
    UserEvent,
    LedgerDelta,
}

/// One official WS family. The slice returned by [`families`] is the list, not a `match`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WsFamily {
    pub identifier: &'static str,
    pub capability_id: &'static str,
    pub channel: &'static str,
    pub user_scoped: bool,
    pub coin_scoped: bool,
    pub requires_interval: bool,
    pub snapshot_policy: SnapshotPolicy,
    pub payload_shape: PayloadShape,
    pub data_array_field: Option<&'static str>,
    pub snapshot_class: ObservationClass,
    pub incremental_class: ObservationClass,
    pub state_affecting: bool,
    pub variant_classifier: VariantClassifier,
}

const FAMILIES: &[WsFamily] = &[
    WsFamily {
        identifier: "allMids",
        capability_id: "official.ws.all_mids",
        channel: "allMids",
        user_scoped: false,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::FullReplace,
        payload_shape: PayloadShape::Object,
        data_array_field: None,
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::Snapshot,
        state_affecting: false,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "notification",
        capability_id: "official.ws.notification",
        channel: "notification",
        user_scoped: true,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::Incremental,
        payload_shape: PayloadShape::Object,
        data_array_field: None,
        snapshot_class: ObservationClass::ProvisionalFeed,
        incremental_class: ObservationClass::ProvisionalFeed,
        state_affecting: false,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "webData3",
        capability_id: "official.ws.web_data3",
        channel: "webData3",
        user_scoped: true,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::FullReplace,
        payload_shape: PayloadShape::Object,
        data_array_field: None,
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::Snapshot,
        state_affecting: true,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "twapStates",
        capability_id: "official.ws.twap_states",
        channel: "twapStates",
        user_scoped: true,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::FullReplace,
        payload_shape: PayloadShape::Object,
        data_array_field: None,
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::Snapshot,
        state_affecting: true,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "clearinghouseState",
        capability_id: "official.ws.clearinghouse_state",
        channel: "clearinghouseState",
        user_scoped: true,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::FullReplace,
        payload_shape: PayloadShape::Object,
        data_array_field: None,
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::Snapshot,
        state_affecting: true,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "openOrders",
        capability_id: "official.ws.open_orders",
        channel: "openOrders",
        user_scoped: true,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::FullReplace,
        payload_shape: PayloadShape::Object,
        data_array_field: None,
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::Snapshot,
        state_affecting: true,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "candle",
        capability_id: "official.ws.candle",
        channel: "candle",
        user_scoped: false,
        coin_scoped: true,
        requires_interval: true,
        snapshot_policy: SnapshotPolicy::Incremental,
        payload_shape: PayloadShape::Array,
        data_array_field: None,
        snapshot_class: ObservationClass::PublicMarketData,
        incremental_class: ObservationClass::PublicMarketData,
        state_affecting: false,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "l2Book",
        capability_id: "official.ws.l2_book",
        channel: "l2Book",
        user_scoped: false,
        coin_scoped: true,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::FullReplace,
        payload_shape: PayloadShape::Object,
        data_array_field: None,
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::Snapshot,
        state_affecting: false,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "trades",
        capability_id: "official.ws.trades",
        channel: "trades",
        user_scoped: false,
        coin_scoped: true,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::Incremental,
        payload_shape: PayloadShape::Array,
        data_array_field: None,
        snapshot_class: ObservationClass::PublicMarketData,
        incremental_class: ObservationClass::PublicMarketData,
        state_affecting: false,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "orderUpdates",
        capability_id: "official.ws.order_updates",
        channel: "orderUpdates",
        user_scoped: true,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::Incremental,
        payload_shape: PayloadShape::Array,
        data_array_field: None,
        snapshot_class: ObservationClass::ProvisionalFeed,
        incremental_class: ObservationClass::ProvisionalFeed,
        state_affecting: true,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "userEvents",
        capability_id: "official.ws.user_events",
        channel: "user",
        user_scoped: true,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::Incremental,
        payload_shape: PayloadShape::Object,
        data_array_field: None,
        snapshot_class: ObservationClass::ProvisionalFeed,
        incremental_class: ObservationClass::ProvisionalFeed,
        state_affecting: true,
        variant_classifier: VariantClassifier::UserEvent,
    },
    WsFamily {
        identifier: "userFills",
        capability_id: "official.ws.user_fills",
        channel: "userFills",
        user_scoped: true,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::Tagged,
        payload_shape: PayloadShape::Object,
        data_array_field: Some("fills"),
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::ProvisionalFeed,
        state_affecting: true,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "userFundings",
        capability_id: "official.ws.user_fundings",
        channel: "userFundings",
        user_scoped: true,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::Tagged,
        payload_shape: PayloadShape::Object,
        data_array_field: Some("fundings"),
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::ProvisionalFeed,
        state_affecting: true,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "userNonFundingLedgerUpdates",
        capability_id: "official.ws.user_non_funding_ledger_updates",
        channel: "userNonFundingLedgerUpdates",
        user_scoped: true,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::Tagged,
        payload_shape: PayloadShape::Object,
        data_array_field: Some("nonFundingLedgerUpdates"),
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::ProvisionalFeed,
        state_affecting: true,
        variant_classifier: VariantClassifier::LedgerDelta,
    },
    WsFamily {
        identifier: "activeAssetCtx",
        capability_id: "official.ws.active_asset_ctx",
        channel: "activeAssetCtx",
        user_scoped: false,
        coin_scoped: true,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::FullReplace,
        payload_shape: PayloadShape::Object,
        data_array_field: None,
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::Snapshot,
        state_affecting: false,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "activeAssetData",
        capability_id: "official.ws.active_asset_data",
        channel: "activeAssetData",
        user_scoped: true,
        coin_scoped: true,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::FullReplace,
        payload_shape: PayloadShape::Object,
        data_array_field: None,
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::Snapshot,
        state_affecting: true,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "userTwapSliceFills",
        capability_id: "official.ws.user_twap_slice_fills",
        channel: "userTwapSliceFills",
        user_scoped: true,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::Tagged,
        payload_shape: PayloadShape::Object,
        data_array_field: Some("twapSliceFills"),
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::ProvisionalFeed,
        state_affecting: true,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "userTwapHistory",
        capability_id: "official.ws.user_twap_history",
        channel: "userTwapHistory",
        user_scoped: true,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::Tagged,
        payload_shape: PayloadShape::Object,
        data_array_field: Some("history"),
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::ProvisionalFeed,
        state_affecting: true,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "bbo",
        capability_id: "official.ws.bbo",
        channel: "bbo",
        user_scoped: false,
        coin_scoped: true,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::Incremental,
        payload_shape: PayloadShape::Object,
        data_array_field: None,
        snapshot_class: ObservationClass::PublicMarketData,
        incremental_class: ObservationClass::PublicMarketData,
        state_affecting: false,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "spotState",
        capability_id: "official.ws.spot_state",
        channel: "spotState",
        user_scoped: true,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::FullReplace,
        payload_shape: PayloadShape::Object,
        data_array_field: None,
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::Snapshot,
        state_affecting: true,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "allDexsClearinghouseState",
        capability_id: "official.ws.all_dexs_clearinghouse_state",
        channel: "allDexsClearinghouseState",
        user_scoped: true,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::FullReplace,
        payload_shape: PayloadShape::Object,
        data_array_field: None,
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::Snapshot,
        state_affecting: true,
        variant_classifier: VariantClassifier::None,
    },
    WsFamily {
        identifier: "allDexsAssetCtxs",
        capability_id: "official.ws.all_dexs_asset_ctxs",
        channel: "allDexsAssetCtxs",
        user_scoped: false,
        coin_scoped: false,
        requires_interval: false,
        snapshot_policy: SnapshotPolicy::FullReplace,
        payload_shape: PayloadShape::Object,
        data_array_field: None,
        snapshot_class: ObservationClass::Snapshot,
        incremental_class: ObservationClass::Snapshot,
        state_affecting: false,
        variant_classifier: VariantClassifier::None,
    },
];

#[must_use]
pub const fn families() -> &'static [WsFamily] {
    FAMILIES
}

/// ponytail: O(n) scan over 22 rows. HashMap if T11 parses a hot path.
#[must_use]
pub fn family_by_identifier(identifier: &str) -> Option<&'static WsFamily> {
    FAMILIES
        .iter()
        .find(|family| family.identifier == identifier)
}

#[must_use]
pub fn family_by_channel(channel: &str) -> Option<&'static WsFamily> {
    FAMILIES.iter().find(|family| family.channel == channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_family_identifiers_and_channels_are_unique() {
        let families = families();
        let identifiers = families
            .iter()
            .map(|family| family.identifier)
            .collect::<std::collections::BTreeSet<_>>();
        let channels = families
            .iter()
            .map(|family| family.channel)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(identifiers.len(), families.len());
        assert_eq!(channels.len(), families.len());
        assert_eq!(families.len(), 22);
    }

    #[test]
    fn user_events_wire_channel_is_user() {
        let family = family_by_identifier("userEvents").expect("family");
        assert_eq!(family.channel, "user");
        assert_eq!(
            family_by_channel("user").map(|row| row.identifier),
            Some("userEvents")
        );
    }

    #[test]
    fn no_family_uses_committed_observation_class() {
        for family in families() {
            assert_ne!(family.snapshot_class, ObservationClass::CommittedBlock);
            assert_ne!(family.incremental_class, ObservationClass::CommittedBlock);
        }
    }
}
