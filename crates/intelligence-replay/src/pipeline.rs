use std::collections::{BTreeMap, BTreeSet};

use canonical_events::{
    BlockEnvelope, CommittedNodeV1MappingContext, ConfirmationClass, map_committed_node_v1_block,
};
use canonical_ledger::{
    AccountFactRecordV1, AccountVaultRelationCurrentRecordV1, ApplyOutcome, CanonicalLedger,
    CanonicalStateReducerV1, EventReducer, LedgerLimits, MarketCurrentRecordV1, MarketStatusV1,
    StateImage, SubaccountMasterCurrentRecordV1, WatermarkOnlyReducerV1,
};
use domain_types::{
    AccountId, Address, BlockHeight, EvidenceId, FeatureSetVersion, Horizon, KnownTime,
    ProtocolTime, ScenarioId, UsdAmount,
};
use entity_graph::{EntityGraph, GraphNodeId, LinkEvidence, LinkKind};
use feature_core::{
    require_asof, EvidenceKind, EvidenceRef, FeatureCalculator, FeatureContext, FeatureDelta,
    FeatureKey, FeatureSnapshot, FeatureSubject, FeatureValue, HealthAssessment, HealthState,
    MissingReason, PitSnapshotCalculator,
};
use hl_protocol::node::v1::NodeRecordV1;
use market_intelligence::{
    FragilityScenario, MarketError, MarketFeatureSnapshot, crowding_components_from_snapshot,
    market_feature_key, simulate_fragility_from_snapshot,
};
use signal_core::{ProofWithholdReason, SignalConfirmationClass, proof_withhold_reason};
use wallet_intelligence::{
    DEFAULT_RETURN_SCALE, DEFAULT_USD_SCALE, IntelligenceError, IntelligenceSubject,
    PerformanceLedger,
};

use crate::IntelligenceReplayError;

const ACCOUNT_FACT_NS: &str = "account-fact.v1";
const SUBACCOUNT_NS: &str = "account-subaccount-master.v1";
const VAULT_RELATION_NS: &str = "account-vault-relation.v1";
const MARKET_CURRENT_NS: &str = "market-current.v1";
const SOURCE_QUALIFICATION: &str = "synthetic_unassessed";

/// Explicit qualification claim. Only synthetic-unassessed replay is admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualificationClaim {
    SyntheticUnassessed,
    LiveQualified,
    Stage3Pass,
    Alpha,
}

impl QualificationClaim {
    pub fn admit(self) -> Result<(), IntelligenceReplayError> {
        match self {
            Self::SyntheticUnassessed => Ok(()),
            Self::LiveQualified => Err(IntelligenceReplayError::QualificationClaim {
                what: "live_qualified",
            }),
            Self::Stage3Pass => Err(IntelligenceReplayError::QualificationClaim {
                what: "stage_3_pass",
            }),
            Self::Alpha => Err(IntelligenceReplayError::QualificationClaim { what: "alpha" }),
        }
    }

    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::SyntheticUnassessed => SOURCE_QUALIFICATION,
            Self::LiveQualified => "live_qualified",
            Self::Stage3Pass => "stage_3_pass",
            Self::Alpha => "alpha",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializeRequest {
    pub qualification: QualificationClaim,
    pub feature_set_version: FeatureSetVersion,
}

impl MaterializeRequest {
    pub fn synthetic_unassessed(
        feature_set_version: FeatureSetVersion,
    ) -> Result<Self, IntelligenceReplayError> {
        QualificationClaim::SyntheticUnassessed.admit()?;
        Ok(Self {
            qualification: QualificationClaim::SyntheticUnassessed,
            feature_set_version,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntelligenceReplayReport {
    pub feature_snapshots: Vec<FeatureSnapshot>,
    pub entity_graph: EntityGraph,
    pub market_snapshots: Vec<MarketFeatureSnapshot>,
    pub input_watermark: BlockHeight,
    pub state_hash: [u8; 32],
    pub source_qualification: &'static str,
    pub stage_3_pass: bool,
    pub live_qualified: bool,
    pub alpha_qualified: bool,
    pub wallet_performance_withheld: bool,
    pub live_signal_count: u64,
    pub crowding_emitted: u64,
    pub fragility_emitted: u64,
    pub fills_invented: bool,
    pub marks_invented: bool,
    pub replica_cmds_used: bool,
    pub signal_confirmation: SignalConfirmationClass,
}

impl IntelligenceReplayReport {
    pub fn require_asof_account(
        &self,
        account: &AccountId,
        effective_at: ProtocolTime,
        known_at: KnownTime,
    ) -> Result<&FeatureSnapshot, IntelligenceReplayError> {
        let subject = FeatureSubject::Account(account.clone());
        let matched: Vec<FeatureSnapshot> = self
            .feature_snapshots
            .iter()
            .filter(|snapshot| snapshot.subject == subject)
            .cloned()
            .collect();
        let visible = require_asof(&matched, effective_at, known_at)?;
        self.feature_snapshots
            .iter()
            .find(|snapshot| snapshot.provenance_hash == visible.provenance_hash)
            .ok_or(IntelligenceReplayError::MissingState)
    }

    pub fn require_wallet_performance(&self) -> Result<(), IntelligenceReplayError> {
        if self.wallet_performance_withheld {
            Err(IntelligenceReplayError::MissingState)
        } else {
            Err(IntelligenceReplayError::QualificationClaim {
                what: "invented_wallet_equity",
            })
        }
    }

    pub fn require_live_signal(&self) -> Result<(), IntelligenceReplayError> {
        if self.live_signal_count == 0 {
            Err(IntelligenceReplayError::QualificationClaim {
                what: "live_signal",
            })
        } else {
            Err(IntelligenceReplayError::QualificationClaim {
                what: "live_qualified",
            })
        }
    }
}

/// Replay generated canonical blocks through in-memory reconstructed state and
/// materialize PIT wallet/entity/market features. This path does not read
/// `replica_cmds`, invent fills, or claim live/Stage 3/alpha qualification.
pub fn materialize_synthetic_replay(
    blocks: &[BlockEnvelope],
    request: &MaterializeRequest,
) -> Result<IntelligenceReplayReport, IntelligenceReplayError> {
    request.qualification.admit()?;
    let Some(first) = blocks.first() else {
        return Err(IntelligenceReplayError::MissingState);
    };
    let reducer = CanonicalStateReducerV1::try_new()?;
    let mut ledger = CanonicalLedger::try_new(
        first.chain_id().clone(),
        first.block_height(),
        reducer,
        LedgerLimits::production(),
    )?;
    replay_and_materialize(&mut ledger, blocks, request)
}

/// Map a committed node record and materialize intelligence. Action-bearing
/// bundles fail closed while the mapper still rejects them; empty committed
/// blocks produce watermark-only state and fail closed for missing intelligence.
pub fn materialize_committed_node(
    record: &NodeRecordV1,
    context: &CommittedNodeV1MappingContext,
    request: &MaterializeRequest,
) -> Result<IntelligenceReplayReport, IntelligenceReplayError> {
    request.qualification.admit()?;
    let block = map_committed_node_v1_block(record, context)?;
    if !block.events().is_empty() {
        return Err(IntelligenceReplayError::ActionBearingRejected {
            action_bundles: block.events().len(),
        });
    }
    let mut ledger = CanonicalLedger::try_new(
        block.chain_id().clone(),
        block.block_height(),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
    )?;
    replay_and_materialize(&mut ledger, std::slice::from_ref(&block), request)
}

fn replay_and_materialize<R: EventReducer>(
    ledger: &mut CanonicalLedger<R>,
    blocks: &[BlockEnvelope],
    request: &MaterializeRequest,
) -> Result<IntelligenceReplayReport, IntelligenceReplayError> {
    let mut calculator = PitSnapshotCalculator::new();
    let mut block_times = BTreeMap::new();
    for block in blocks {
        if !matches!(
            block.confirmation_class(),
            ConfirmationClass::CommittedPrimary | ConfirmationClass::CommittedIndependent
        ) {
            return Err(IntelligenceReplayError::Ledger {
                reason_code: "ledger.non_committed_block",
            });
        }
        match ledger.apply_block(block)? {
            ApplyOutcome::Applied(_) => {}
            ApplyOutcome::AlreadyApplied(_) => {
                return Err(IntelligenceReplayError::Ledger {
                    reason_code: "ledger.canonical_divergence",
                });
            }
        }
        block_times.insert(block.block_height(), block.block_time());
        let ctx = context_for(request, block.block_time(), block.block_height())?;
        emit_snapshots(ledger.state_image(), &ctx, &mut calculator)?;
    }
    let watermark = ledger
        .checkpoint()
        .ok_or(IntelligenceReplayError::MissingState)?
        .block_height();
    let last_time = block_times
        .get(&watermark)
        .copied()
        .ok_or(IntelligenceReplayError::MissingState)?;
    let ctx = context_for(request, last_time, watermark)?;
    let facts = decode_facts(ledger.state_image())?;
    if facts.is_empty() || calculator.snapshots().is_empty() {
        return Err(IntelligenceReplayError::MissingState);
    }
    let entity_graph = emit_links(&facts, &block_times)?;
    let market_snapshots = emit_market_snapshots(&facts, &ctx)?;
    let wallet_performance_withheld = withhold_wallet_performance(&facts.accounts, &ctx)?;
    let (crowding_emitted, fragility_emitted, live_signal_count) =
        assess_market_emissions(&market_snapshots)?;

    Ok(IntelligenceReplayReport {
        feature_snapshots: calculator.snapshots().to_vec(),
        entity_graph,
        market_snapshots,
        input_watermark: watermark,
        state_hash: ledger.state_hash(),
        source_qualification: SOURCE_QUALIFICATION,
        stage_3_pass: false,
        live_qualified: false,
        alpha_qualified: false,
        wallet_performance_withheld,
        live_signal_count,
        crowding_emitted,
        fragility_emitted,
        fills_invented: false,
        marks_invented: false,
        replica_cmds_used: false,
        signal_confirmation: SignalConfirmationClass::SyntheticUnqualified,
    })
}

fn context_for(
    request: &MaterializeRequest,
    time: ProtocolTime,
    height: BlockHeight,
) -> Result<FeatureContext, IntelligenceReplayError> {
    Ok(FeatureContext::try_new(
        request.feature_set_version.clone(),
        time,
        to_known(time)?,
        height,
        HealthState::Amber,
    )?)
}

fn to_known(time: ProtocolTime) -> Result<KnownTime, IntelligenceReplayError> {
    Ok(KnownTime::from_unix_micros(time.unix_micros())?)
}

#[derive(Debug, Default)]
struct ReconstructedFacts {
    accounts: BTreeSet<Address>,
    subaccounts: Vec<SubaccountMasterCurrentRecordV1>,
    vaults: Vec<AccountVaultRelationCurrentRecordV1>,
    markets: Vec<MarketCurrentRecordV1>,
}

impl ReconstructedFacts {
    fn is_empty(&self) -> bool {
        self.accounts.is_empty()
            && self.subaccounts.is_empty()
            && self.vaults.is_empty()
            && self.markets.is_empty()
    }
}

fn decode_facts(image: &StateImage) -> Result<ReconstructedFacts, IntelligenceReplayError> {
    let mut facts = ReconstructedFacts::default();
    for (key, value) in image.entries() {
        match key.namespace() {
            ACCOUNT_FACT_NS => {
                let record = AccountFactRecordV1::decode_at(key, value)?;
                facts.accounts.extend(record.account_ids().iter().copied());
            }
            SUBACCOUNT_NS => {
                let record = SubaccountMasterCurrentRecordV1::decode_at(key, value)?;
                facts.accounts.insert(record.master_account_id());
                facts.accounts.insert(record.subaccount_id());
                facts.subaccounts.push(record);
            }
            VAULT_RELATION_NS => {
                let record = AccountVaultRelationCurrentRecordV1::decode_at(key, value)?;
                facts.accounts.insert(record.account_id());
                facts.vaults.push(record);
            }
            MARKET_CURRENT_NS => {
                facts
                    .markets
                    .push(MarketCurrentRecordV1::decode_at(key, value)?);
            }
            _ => {}
        }
    }
    Ok(facts)
}

fn emit_snapshots(
    image: &StateImage,
    ctx: &FeatureContext,
    calculator: &mut PitSnapshotCalculator,
) -> Result<(), IntelligenceReplayError> {
    let facts = decode_facts(image)?;
    for account in &facts.accounts {
        let delta = account_delta(account)?;
        calculator.on_delta(&delta, ctx, None)?;
    }
    for market in &facts.markets {
        let delta = market_registry_delta(market)?;
        calculator.on_delta(&delta, ctx, None)?;
    }
    Ok(())
}

fn account_delta(account: &Address) -> Result<FeatureDelta, IntelligenceReplayError> {
    let mut values = BTreeMap::new();
    values.insert(
        feature_key("wallet", "reconstructed")?,
        FeatureValue::Boolean(true),
    );
    values.insert(
        feature_key("wallet", "equity_usd")?,
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    values.insert(
        feature_key("wallet", "fills")?,
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    Ok(FeatureDelta::try_new(
        FeatureSubject::Account(AccountId::new(account.to_api_string())?),
        values,
    )?)
}

fn market_registry_delta(
    record: &MarketCurrentRecordV1,
) -> Result<FeatureDelta, IntelligenceReplayError> {
    let mut values = BTreeMap::new();
    values.insert(
        feature_key("market", "registry")?,
        FeatureValue::Boolean(true),
    );
    values.insert(
        feature_key("market", "status")?,
        FeatureValue::try_category(market_status_name(record.status()))?,
    );
    values.insert(
        feature_key("market", "book")?,
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    values.insert(
        feature_key("market", "fills")?,
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    Ok(FeatureDelta::try_new(
        FeatureSubject::Market(record.market_id().clone()),
        values,
    )?)
}

const fn market_status_name(status: MarketStatusV1) -> &'static str {
    match status {
        MarketStatusV1::Active => "active",
        MarketStatusV1::Halted => "halted",
    }
}

fn feature_key(namespace: &str, name: &str) -> Result<FeatureKey, IntelligenceReplayError> {
    Ok(FeatureKey::try_new(namespace, name, 1)?)
}

fn emit_links(
    facts: &ReconstructedFacts,
    block_times: &BTreeMap<BlockHeight, ProtocolTime>,
) -> Result<EntityGraph, IntelligenceReplayError> {
    let mut graph = EntityGraph::new();
    for record in &facts.subaccounts {
        let effective_at = time_at(block_times, record.first_block_height())?;
        let known_at = to_known(effective_at)?;
        let evidence = state_evidence(
            record.last_event_id(),
            record_digest(record.last_event_id().as_str().as_bytes(), effective_at),
            effective_at,
            known_at,
        )?;
        graph.insert_link(LinkEvidence::try_new(
            EvidenceId::new(record.last_event_id().as_str())?,
            GraphNodeId::Account(AccountId::new(record.master_account_id().to_api_string())?),
            GraphNodeId::Account(AccountId::new(record.subaccount_id().to_api_string())?),
            LinkKind::ProtocolSubaccount,
            domain_types::ProbabilityPpm::ONE,
            effective_at,
            known_at,
            vec![evidence],
            None,
        )?)?;
    }
    for record in &facts.vaults {
        let effective_at = time_at(block_times, record.first_block_height())?;
        let known_at = to_known(effective_at)?;
        let evidence = state_evidence(
            record.last_event_id(),
            record_digest(record.last_event_id().as_str().as_bytes(), effective_at),
            effective_at,
            known_at,
        )?;
        graph.insert_link(LinkEvidence::try_new(
            EvidenceId::new(record.last_event_id().as_str())?,
            GraphNodeId::Account(AccountId::new(record.account_id().to_api_string())?),
            GraphNodeId::Vault(record.vault_id().clone()),
            LinkKind::ProtocolVaultMembership,
            domain_types::ProbabilityPpm::ONE,
            effective_at,
            known_at,
            vec![evidence],
            None,
        )?)?;
    }
    if let Some((&_, &effective_at)) = block_times.iter().next_back() {
        graph.known_administrative_groups(effective_at, to_known(effective_at)?)?;
    }
    Ok(graph)
}

fn time_at(
    block_times: &BTreeMap<BlockHeight, ProtocolTime>,
    height: BlockHeight,
) -> Result<ProtocolTime, IntelligenceReplayError> {
    block_times
        .get(&height)
        .copied()
        .ok_or(IntelligenceReplayError::MissingState)
}

fn record_digest(identity: &[u8], time: ProtocolTime) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(identity);
    hasher.update(&time.unix_micros().to_le_bytes());
    *hasher.finalize().as_bytes()
}

fn state_evidence(
    event_id: &domain_types::EventId,
    content_hash: [u8; 32],
    effective_at: ProtocolTime,
    known_at: KnownTime,
) -> Result<EvidenceRef, IntelligenceReplayError> {
    Ok(EvidenceRef::try_new(
        EvidenceKind::StateSnapshot,
        EvidenceId::new(event_id.as_str())?,
        content_hash,
        effective_at,
        known_at,
    )?)
}

fn emit_market_snapshots(
    facts: &ReconstructedFacts,
    ctx: &FeatureContext,
) -> Result<Vec<MarketFeatureSnapshot>, IntelligenceReplayError> {
    let mut snapshots = Vec::new();
    for record in &facts.markets {
        let mut values = BTreeMap::new();
        values.insert(market_feature_key("registry")?, FeatureValue::Boolean(true));
        values.insert(
            market_feature_key("book")?,
            FeatureValue::Missing(MissingReason::NotObserved),
        );
        values.insert(
            market_feature_key("fills")?,
            FeatureValue::Missing(MissingReason::NotObserved),
        );
        values.insert(
            market_feature_key("inventory")?,
            FeatureValue::Missing(MissingReason::NotObserved),
        );
        snapshots.push(MarketFeatureSnapshot::try_new(
            record.market_id().clone(),
            Horizon::MINUTES_5,
            ctx.feature_set_version.clone(),
            ctx.effective_at,
            ctx.known_at,
            ctx.input_watermark,
            values,
            HealthAssessment::try_new("market", HealthState::Amber, SOURCE_QUALIFICATION)?,
        )?);
    }
    Ok(snapshots)
}

fn withhold_wallet_performance(
    accounts: &BTreeSet<Address>,
    ctx: &FeatureContext,
) -> Result<bool, IntelligenceReplayError> {
    let mut withheld = accounts.is_empty();
    for account in accounts {
        let ledger = PerformanceLedger::try_new(
            IntelligenceSubject::Account(AccountId::new(account.to_api_string())?),
            DEFAULT_USD_SCALE,
            DEFAULT_RETURN_SCALE,
        )?;
        match ledger.snapshot(
            ctx.feature_set_version.clone(),
            ctx.known_at,
            ctx.input_watermark,
            Some(ctx.effective_at),
        ) {
            Err(IntelligenceError::InsufficientHistory { .. }) => withheld = true,
            Err(error) => return Err(error.into()),
            Ok(_) => {
                return Err(IntelligenceReplayError::QualificationClaim {
                    what: "invented_wallet_equity",
                });
            }
        }
    }
    Ok(withheld)
}

fn assess_market_emissions(
    snapshots: &[MarketFeatureSnapshot],
) -> Result<(u64, u64, u64), IntelligenceReplayError> {
    let remaining_capacity = UsdAmount::from_raw(0, 8)?;
    let scenario =
        FragilityScenario::default_grid(ScenarioId::new("synthetic-unassessed-fragility")?);
    let mut crowding_emitted = 0_u64;
    let mut fragility_emitted = 0_u64;
    let mut missing_book_or_fills = false;
    let mut inventory_withheld = false;
    for snapshot in snapshots {
        match proof_withhold_reason(snapshot) {
            Some(ProofWithholdReason::MissingBookOrFills) => missing_book_or_fills = true,
            Some(ProofWithholdReason::MissingInventory)
            | Some(ProofWithholdReason::MalformedInventory) => {
                inventory_withheld = true;
            }
            None => {}
        }
        match crowding_components_from_snapshot(snapshot, &[], remaining_capacity) {
            Ok(_) => crowding_emitted = crowding_emitted.saturating_add(1),
            Err(error) if withheld_market_emission(&error) => {}
            Err(error) => return Err(error.into()),
        }
        match simulate_fragility_from_snapshot(snapshot, &scenario, &[], 0) {
            Ok(result) => {
                if result.missing_inputs.is_empty()
                    && (!result.base.waves.is_empty()
                        || result.base.total_forced_notional.raw() != 0)
                {
                    fragility_emitted = fragility_emitted.saturating_add(1);
                }
            }
            Err(error) if withheld_market_emission(&error) => {}
            Err(error) => return Err(error.into()),
        }
    }
    let live_signal_count = 0_u64;
    if (missing_book_or_fills || inventory_withheld)
        && (crowding_emitted > 0 || fragility_emitted > 0 || live_signal_count > 0)
    {
        return Err(IntelligenceReplayError::QualificationClaim {
            what: "invented_marks_or_fills",
        });
    }
    Ok((crowding_emitted, fragility_emitted, live_signal_count))
}

fn withheld_market_emission(error: &MarketError) -> bool {
    match error {
        MarketError::MissingInput {
            name: "book" | "fills" | "inventory",
        } => true,
        MarketError::Malformed {
            what: "inventory", ..
        } => true,
        MarketError::InsufficientHistory { .. } => true,
        MarketError::MissingInput { .. }
        | MarketError::Malformed { .. }
        | MarketError::EmptyIdentifier { .. }
        | MarketError::Unsupported { .. }
        | MarketError::RedDataHealth { .. }
        | MarketError::EmptyDenominator
        | MarketError::ScaleMismatch
        | MarketError::Overflow
        | MarketError::DivisionByZero
        | MarketError::OutOfRange
        | MarketError::Feature(_) => false,
    }
}
