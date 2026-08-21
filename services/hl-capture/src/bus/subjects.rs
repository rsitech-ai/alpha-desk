use canonical_events::EventKind;

pub const CANONICAL_STREAM: &str = "HL_CANONICAL";
pub const STATE_STREAM: &str = "HL_STATE";
pub const FEATURE_STREAM: &str = "HL_FEATURE";
pub const SIGNAL_STREAM: &str = "HL_SIGNAL";
pub const HEALTH_STREAM: &str = "HL_HEALTH";
pub const DEAD_LETTER_STREAM: &str = "HL_DEADLETTER";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Subject {
    BlockCommitted,
    BlockProvisional,
    EventFill,
    EventOrder,
    EventLedger,
    EventMarketMeta,
    EventOracle,
    SnapshotAccount,
    SnapshotMarket,
    SnapshotEcosystem,
    StateAccountDelta,
    StateBookDelta,
    FeatureWallet,
    FeatureEntity,
    FeatureMarket,
    SignalCandidate,
    SignalLive,
    SignalResolved,
    HealthData,
    HealthSource,
    HealthModel,
}

impl Subject {
    pub const ALL: [Self; 21] = [
        Self::BlockCommitted,
        Self::BlockProvisional,
        Self::EventFill,
        Self::EventOrder,
        Self::EventLedger,
        Self::EventMarketMeta,
        Self::EventOracle,
        Self::SnapshotAccount,
        Self::SnapshotMarket,
        Self::SnapshotEcosystem,
        Self::StateAccountDelta,
        Self::StateBookDelta,
        Self::FeatureWallet,
        Self::FeatureEntity,
        Self::FeatureMarket,
        Self::SignalCandidate,
        Self::SignalLive,
        Self::SignalResolved,
        Self::HealthData,
        Self::HealthSource,
        Self::HealthModel,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BlockCommitted => "hl.v1.block.committed",
            Self::BlockProvisional => "hl.v1.block.provisional",
            Self::EventFill => "hl.v1.event.fill",
            Self::EventOrder => "hl.v1.event.order",
            Self::EventLedger => "hl.v1.event.ledger",
            Self::EventMarketMeta => "hl.v1.event.market_meta",
            Self::EventOracle => "hl.v1.event.oracle",
            Self::SnapshotAccount => "hl.v1.snapshot.account",
            Self::SnapshotMarket => "hl.v1.snapshot.market",
            Self::SnapshotEcosystem => "hl.v1.snapshot.ecosystem",
            Self::StateAccountDelta => "hl.v1.state.account_delta",
            Self::StateBookDelta => "hl.v1.state.book_delta",
            Self::FeatureWallet => "hl.v1.feature.wallet",
            Self::FeatureEntity => "hl.v1.feature.entity",
            Self::FeatureMarket => "hl.v1.feature.market",
            Self::SignalCandidate => "hl.v1.signal.candidate",
            Self::SignalLive => "hl.v1.signal.live",
            Self::SignalResolved => "hl.v1.signal.resolved",
            Self::HealthData => "hl.v1.health.data",
            Self::HealthSource => "hl.v1.health.source",
            Self::HealthModel => "hl.v1.health.model",
        }
    }

    #[must_use]
    pub const fn stream(self) -> &'static str {
        match self {
            Self::BlockCommitted
            | Self::BlockProvisional
            | Self::EventFill
            | Self::EventOrder
            | Self::EventLedger
            | Self::EventMarketMeta
            | Self::EventOracle
            | Self::SnapshotAccount
            | Self::SnapshotMarket
            | Self::SnapshotEcosystem => CANONICAL_STREAM,
            Self::StateAccountDelta | Self::StateBookDelta => STATE_STREAM,
            Self::FeatureWallet | Self::FeatureEntity | Self::FeatureMarket => FEATURE_STREAM,
            Self::SignalCandidate | Self::SignalLive | Self::SignalResolved => SIGNAL_STREAM,
            Self::HealthData | Self::HealthSource | Self::HealthModel => HEALTH_STREAM,
        }
    }
}

#[must_use]
pub const fn subject_for_event_kind(kind: EventKind) -> Subject {
    match kind {
        EventKind::OrderPartiallyFilled
        | EventKind::OrderFilled
        | EventKind::TwapSliceFilled
        | EventKind::TradeMatched
        | EventKind::LiquidationFill
        | EventKind::BackstopLiquidation => Subject::EventFill,

        EventKind::OrderAccepted
        | EventKind::OrderRested
        | EventKind::OrderModified
        | EventKind::OrderCancelled
        | EventKind::OrderRejected
        | EventKind::TriggerOrderActivated
        | EventKind::TwapStarted
        | EventKind::TwapCompleted => Subject::EventOrder,

        EventKind::OracleUpdated | EventKind::FundingRateUpdated => Subject::EventOracle,

        EventKind::MarketHalted
        | EventKind::MarketResumed
        | EventKind::OpenInterestCapChanged
        | EventKind::MarginTableChanged
        | EventKind::MarketCreated
        | EventKind::MarketMetadataChanged
        | EventKind::AssetContextUpdated
        | EventKind::DexCreated
        | EventKind::OutcomeCreated
        | EventKind::OutcomeResolved => Subject::EventMarketMeta,

        EventKind::DepositCredited
        | EventKind::WithdrawalDebited
        | EventKind::SpotTransfer
        | EventKind::PerpTransfer
        | EventKind::SubaccountTransfer
        | EventKind::VaultDeposit
        | EventKind::VaultWithdrawal
        | EventKind::FeeCharged
        | EventKind::BuilderFeeCharged
        | EventKind::FundingPaid
        | EventKind::FundingReceived
        | EventKind::ReferralReward
        | EventKind::AccountModeChanged
        | EventKind::MarginModeChanged
        | EventKind::LeverageChanged
        | EventKind::LiquidationStarted
        | EventKind::PositionSettled => Subject::EventLedger,
    }
}
