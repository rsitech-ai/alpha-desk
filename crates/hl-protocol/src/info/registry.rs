use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::{ObservationClass, SourceAdmission, SourceTrust};

use super::{
    EncodedInfoRequest, InfoError, InfoParseContext, ParsedInfoResponse, encode_info_request,
    parse_info_response,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InfoPagination {
    SinglePage,
    ByTime,
}

impl InfoPagination {
    #[must_use]
    pub const fn as_manifest_str(self) -> &'static str {
        match self {
            Self::SinglePage => "none",
            Self::ByTime => "by_time",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InfoStateTarget {
    CommittedState,
    ReconciledSnapshot,
    CanonicalState,
    L4Book,
    PositionState,
    CanonicalEvent,
    ReferenceSnapshot,
    EvmFact,
    DiscoveryOnly,
}

impl InfoStateTarget {
    #[must_use]
    pub const fn as_manifest_str(self) -> &'static str {
        match self {
            Self::CommittedState => "committed_state",
            Self::ReconciledSnapshot => "reconciled_snapshot",
            Self::CanonicalState => "canonical_state",
            Self::L4Book => "l4_book",
            Self::PositionState => "position_state",
            Self::CanonicalEvent => "canonical_event",
            Self::ReferenceSnapshot => "reference_snapshot",
            Self::EvmFact => "evm_fact",
            Self::DiscoveryOnly => "discovery_only",
        }
    }

    #[must_use]
    pub const fn is_state_affecting(self) -> bool {
        match self {
            Self::CommittedState
            | Self::ReconciledSnapshot
            | Self::CanonicalState
            | Self::L4Book
            | Self::PositionState
            | Self::CanonicalEvent
            | Self::EvmFact => true,
            Self::ReferenceSnapshot | Self::DiscoveryOnly => false,
        }
    }

    pub fn from_manifest_str(value: &str) -> Result<Self, InfoError> {
        match value {
            "committed_state" => Ok(Self::CommittedState),
            "reconciled_snapshot" => Ok(Self::ReconciledSnapshot),
            "canonical_state" => Ok(Self::CanonicalState),
            "l4_book" => Ok(Self::L4Book),
            "position_state" => Ok(Self::PositionState),
            "canonical_event" => Ok(Self::CanonicalEvent),
            "reference_snapshot" => Ok(Self::ReferenceSnapshot),
            "evm_fact" => Ok(Self::EvmFact),
            "discovery_only" => Ok(Self::DiscoveryOnly),
            _ => Err(InfoError::MalformedJson),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnsupportedNetwork {
    network: &'static str,
    reason: &'static str,
}

impl UnsupportedNetwork {
    #[must_use]
    pub const fn network(self) -> &'static str {
        self.network
    }

    #[must_use]
    pub const fn reason(self) -> &'static str {
        self.reason
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InfoEndpoint {
    capability_id: &'static str,
    identifier: &'static str,
    domain: &'static str,
    pagination: InfoPagination,
    state_target: InfoStateTarget,
    request_cost: &'static str,
    networks: &'static [&'static str],
    unsupported_networks: &'static [UnsupportedNetwork],
}

impl InfoEndpoint {
    #[must_use]
    pub const fn capability_id(self) -> &'static str {
        self.capability_id
    }

    #[must_use]
    pub const fn identifier(self) -> &'static str {
        self.identifier
    }

    #[must_use]
    pub const fn domain(self) -> &'static str {
        self.domain
    }

    #[must_use]
    pub const fn pagination(self) -> InfoPagination {
        self.pagination
    }

    #[must_use]
    pub const fn state_target(self) -> InfoStateTarget {
        self.state_target
    }

    #[must_use]
    pub const fn request_cost(self) -> &'static str {
        self.request_cost
    }

    #[must_use]
    pub const fn networks(self) -> &'static [&'static str] {
        self.networks
    }

    #[must_use]
    pub const fn unsupported_networks(self) -> &'static [UnsupportedNetwork] {
        self.unsupported_networks
    }

    #[must_use]
    pub const fn observation_class(self) -> ObservationClass {
        ObservationClass::Snapshot
    }

    #[must_use]
    pub const fn source_trust(self) -> SourceTrust {
        SourceTrust::ReconciledSnapshot
    }

    pub fn admission(self) -> Result<SourceAdmission, crate::SourceTrustError> {
        SourceAdmission::new(self.source_trust(), self.observation_class())
    }

    pub fn encode(self, params: &BTreeMap<String, Value>) -> Result<EncodedInfoRequest, InfoError> {
        encode_info_request(self.capability_id, self.identifier, params)
    }

    pub fn parse(
        self,
        raw: &[u8],
        context: &InfoParseContext,
    ) -> Result<ParsedInfoResponse<Value>, InfoError> {
        parse_info_response(
            self.capability_id,
            raw,
            context,
            self.state_target.is_state_affecting(),
        )
    }

    #[must_use]
    pub fn available_on(self, network: &str) -> bool {
        self.networks.contains(&network)
            && !self
                .unsupported_networks
                .iter()
                .any(|row| row.network == network)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfoRegistry {
    endpoints: &'static [InfoEndpoint],
}

impl InfoRegistry {
    pub fn try_new(endpoints: &'static [InfoEndpoint]) -> Result<Self, InfoError> {
        let mut ids = BTreeSet::new();
        let mut identifiers = BTreeSet::new();
        for endpoint in endpoints {
            if !ids.insert(endpoint.capability_id) {
                return Err(InfoError::DuplicateCapability);
            }
            if !identifiers.insert(endpoint.identifier) {
                return Err(InfoError::DuplicateIdentifier);
            }
        }
        Ok(Self { endpoints })
    }

    #[must_use]
    pub const fn official() -> Self {
        Self {
            endpoints: REST_INFO_ENDPOINTS,
        }
    }

    pub fn get(self, capability_id: &str) -> Result<&'static InfoEndpoint, InfoError> {
        self.endpoints
            .iter()
            .find(|endpoint| endpoint.capability_id == capability_id)
            .ok_or(InfoError::UnknownCapability)
    }

    pub fn get_by_identifier(self, identifier: &str) -> Result<&'static InfoEndpoint, InfoError> {
        self.endpoints
            .iter()
            .find(|endpoint| endpoint.identifier == identifier)
            .ok_or(InfoError::UnknownIdentifier)
    }

    pub fn encode(
        self,
        capability_id: &str,
        params: &BTreeMap<String, Value>,
    ) -> Result<EncodedInfoRequest, InfoError> {
        self.get(capability_id)?.encode(params)
    }

    pub fn parse(
        self,
        capability_id: &str,
        raw: &[u8],
        context: &InfoParseContext,
    ) -> Result<ParsedInfoResponse<Value>, InfoError> {
        self.get(capability_id)?.parse(raw, context)
    }

    #[must_use]
    pub const fn endpoints(self) -> &'static [InfoEndpoint] {
        self.endpoints
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.endpoints.len()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.endpoints.is_empty()
    }
}

#[allow(clippy::too_many_arguments)]
const fn endpoint(
    capability_id: &'static str,
    identifier: &'static str,
    domain: &'static str,
    pagination: InfoPagination,
    state_target: InfoStateTarget,
    request_cost: &'static str,
    networks: &'static [&'static str],
    unsupported_networks: &'static [UnsupportedNetwork],
) -> InfoEndpoint {
    InfoEndpoint {
        capability_id,
        identifier,
        domain,
        pagination,
        state_target,
        request_cost,
        networks,
        unsupported_networks,
    }
}

// ponytail: one canonical-JSON codec for every T02 rest_info row. T06/T07 add
// typed TryFrom in sibling files; they must not grow a match here. Typed
// parsers replace this opaque Value when family fixtures exist.
pub const REST_INFO_ENDPOINTS: &[InfoEndpoint] = &[
    endpoint(
        "official.info.all_mids",
        "allMids",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:2",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.open_orders",
        "openOrders",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReconciledSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.frontend_open_orders",
        "frontendOpenOrders",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReconciledSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.user_fills",
        "userFills",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::CanonicalEvent,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.user_fills_by_time",
        "userFillsByTime",
        "account",
        InfoPagination::ByTime,
        InfoStateTarget::CanonicalEvent,
        "base:20 variable:window",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.recent_trades",
        "recentTrades",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::CanonicalEvent,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.user_rate_limit",
        "userRateLimit",
        "system",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.order_status",
        "orderStatus",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:2",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.l2_book",
        "l2Book",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:2",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.candle_snapshot",
        "candleSnapshot",
        "market_data",
        InfoPagination::ByTime,
        InfoStateTarget::ReferenceSnapshot,
        "base:20 variable:window",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.exchange_status",
        "exchangeStatus",
        "system",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:2",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.historical_orders",
        "historicalOrders",
        "account",
        InfoPagination::ByTime,
        InfoStateTarget::ReferenceSnapshot,
        "base:20 variable:window",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.user_twap_slice_fills",
        "userTwapSliceFills",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.user_twap_slice_fills_by_time",
        "userTwapSliceFillsByTime",
        "account",
        InfoPagination::ByTime,
        InfoStateTarget::ReferenceSnapshot,
        "base:20 variable:window",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.twap_history",
        "twapHistory",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.sub_accounts",
        "subAccounts",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.user_to_multi_sig_signers",
        "userToMultiSigSigners",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.portfolio",
        "portfolio",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReconciledSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.referral",
        "referral",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.user_fees",
        "userFees",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.user_role",
        "userRole",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:60",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.user_abstraction",
        "userAbstraction",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.user_dex_abstraction",
        "userDexAbstraction",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.extra_agents",
        "extraAgents",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.approved_builders",
        "approvedBuilders",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.user_vault_equities",
        "userVaultEquities",
        "vault",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.vault_details",
        "vaultDetails",
        "vault",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.delegator_summary",
        "delegatorSummary",
        "staking",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.delegations",
        "delegations",
        "staking",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.delegator_history",
        "delegatorHistory",
        "staking",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.delegator_rewards",
        "delegatorRewards",
        "staking",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.validator_stats",
        "validatorStats",
        "staking",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.aligned_quote_token_info",
        "alignedQuoteTokenInfo",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.borrow_lend_user_state",
        "borrowLendUserState",
        "borrow_lend",
        InfoPagination::SinglePage,
        InfoStateTarget::ReconciledSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.borrow_lend_reserve_state",
        "borrowLendReserveState",
        "borrow_lend",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.all_borrow_lend_reserve_states",
        "allBorrowLendReserveStates",
        "borrow_lend",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.perp_dexs",
        "perpDexs",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.meta",
        "meta",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.meta_and_asset_ctxs",
        "metaAndAssetCtxs",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.all_perp_metas",
        "allPerpMetas",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.clearinghouse_state",
        "clearinghouseState",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReconciledSnapshot,
        "base:2",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.user_funding",
        "userFunding",
        "account",
        InfoPagination::ByTime,
        InfoStateTarget::ReferenceSnapshot,
        "base:20 variable:window",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.user_non_funding_ledger_updates",
        "userNonFundingLedgerUpdates",
        "account",
        InfoPagination::ByTime,
        InfoStateTarget::CanonicalEvent,
        "base:20 variable:window",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.non_user_funding_updates",
        "nonUserFundingUpdates",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::CanonicalEvent,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.funding_history",
        "fundingHistory",
        "market_data",
        InfoPagination::ByTime,
        InfoStateTarget::ReferenceSnapshot,
        "base:20 variable:window",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.predicted_fundings",
        "predictedFundings",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.perps_at_open_interest_cap",
        "perpsAtOpenInterestCap",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.perp_deploy_auction_status",
        "perpDeployAuctionStatus",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.active_asset_data",
        "activeAssetData",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReconciledSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.perp_dex_limits",
        "perpDexLimits",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.perp_dex_status",
        "perpDexStatus",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.perp_annotation",
        "perpAnnotation",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.perp_categories",
        "perpCategories",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.perp_concise_annotations",
        "perpConciseAnnotations",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.spot_meta",
        "spotMeta",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.spot_meta_and_asset_ctxs",
        "spotMetaAndAssetCtxs",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.spot_clearinghouse_state",
        "spotClearinghouseState",
        "account",
        InfoPagination::SinglePage,
        InfoStateTarget::ReconciledSnapshot,
        "base:2",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.spot_deploy_state",
        "spotDeployState",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.spot_pair_deploy_auction_status",
        "spotPairDeployAuctionStatus",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.token_details",
        "tokenDetails",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["mainnet", "testnet"],
        &[],
    ),
    endpoint(
        "official.info.outcome_meta",
        "outcomeMeta",
        "market_data",
        InfoPagination::SinglePage,
        InfoStateTarget::ReferenceSnapshot,
        "base:20",
        &["testnet"],
        &[UnsupportedNetwork {
            network: "mainnet",
            reason: "official docs currently document outcomeMeta as testnet-only",
        }],
    ),
];
