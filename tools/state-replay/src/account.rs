use std::{path::PathBuf, str::FromStr, time::Instant};

use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use canonical_events::{
    AccountModeChanged, AssetContextUpdated, BlockEnvelope, BuilderFeeCharged,
    CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, DepositCredited, DexCreated,
    EventKind, EventPayload, FeeCharged, FundingPaid, FundingReceived, MarginModeChanged,
    MarketCreated, OrderAccepted, PerpTransfer, ReferralReward, SourceEvidence, SpotTransfer,
    SubaccountTransfer, TradeMatched, TradeParticipantRoleV1, TradeParticipantV1, VaultDeposit,
    VaultWithdrawal, WithdrawalDebited,
};
use canonical_ledger::{
    AccountFactRecordV1, AccountModeCurrentRecordV1, AccountQuantityFlowCurrentRecordV1,
    AccountQuantityFlowScopeV1, AccountQuoteFlowCurrentRecordV1, AccountQuoteFlowScopeV1,
    AccountStateError, AccountVaultRelationCurrentRecordV1, CanonicalLedger,
    CanonicalStateReducerV1, CheckpointArtifact, CheckpointCompatibility, LedgerLimits,
    LeverageCurrentRecordV1, MarginModeCurrentRecordV1, StateImageLimits,
    SubaccountMasterCurrentRecordV1, VaultPrincipalFlowCurrentRecordV1,
    VaultShareFlowCurrentRecordV1,
};
use canonical_state_store::LocalCheckpointStore;
use domain_types::{
    AccountAbstractionModeV1, Address, AssetId, BlockHeight, ChainId, DexId, FeeRate, FeeTypeV1,
    FundingRate, KnownTime, Leverage, MarginModeV1, MarketId, OrderId, OrderSide, PositionQuantity,
    Price, Quantity, QuoteAmount, SourceId, TradeId, TransactionId, VaultId,
};
use replay_engine::{ReplayLimits, ReplayOutcome, SerialReplayEngine};
use serde::Serialize;
use storage_ports::{CanonicalArchive, StateCheckpointStore};

use super::{
    CHAIN, FIXTURE_EPOCH_MICROS, FixtureRunError, NeverCancel, REPORT_FILE, RejectionReport,
    START_HEIGHT, create_private_output_root, fixture_time, harden_private_tree, publish_report,
    rejection_report, replay_request, source_hashes, validate_replay_counts,
};

const ACCOUNT_REPORT_SCHEMA: &str = "hyperliquid-alpha-desk/state-replay-account-e2e-report/v1";
const ACCOUNT_EVIDENCE_CLASS: &str = "synthetic_canonical_account";
const BUYER: Address = Address::from_bytes([0x11; 20]);
const SELLER: Address = Address::from_bytes([0x22; 20]);
const BUILDER: Address = Address::from_bytes([0x33; 20]);
const REFERRER: Address = Address::from_bytes([0x44; 20]);
const OPERATOR: Address = Address::from_bytes([0x55; 20]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRunConfig {
    pub output: PathBuf,
    pub blocks: u64,
    pub checkpoint_after: u64,
    pub iterations: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountEvidence {
    pub report_path: PathBuf,
}

pub fn run_account_e2e(config: &AccountRunConfig) -> Result<AccountEvidence, FixtureRunError> {
    if config.iterations < 2 {
        return Err(FixtureRunError::InvalidConfig);
    }
    validate_replay_counts(
        config.blocks,
        config.checkpoint_after,
        config.iterations,
        1,
        4,
    )?;
    let output = create_private_output_root(&config.output)?;
    let archive = LocalParquetArchive::open(
        output.join("archive"),
        ArchiveConfig::deterministic_fixture(
            "state-replay-account-e2e-v1",
            KnownTime::from_unix_micros(FIXTURE_EPOCH_MICROS)?,
        )?,
    )?;
    let chain = ChainId::new(CHAIN)?;
    let end_height = START_HEIGHT
        .checked_add(config.blocks - 1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let mut manifests = Vec::new();
    for height in START_HEIGHT..=end_height {
        let block = if height == START_HEIGHT {
            market_prerequisite_block(height, &chain)?
        } else if height == end_height {
            account_flow_block(height, &chain, "1.0.0")?
        } else {
            block(height, &chain, Vec::new())?
        };
        manifests.push(archive.append_block(&block)?.manifest_id().clone());
    }
    let schema_fingerprint = *archive
        .verify_manifest(
            manifests
                .first()
                .ok_or(FixtureRunError::Invariant("missing manifest"))?,
        )?
        .schema_fingerprints()
        .get("canonical_events")
        .ok_or(FixtureRunError::Invariant(
            "missing canonical schema fingerprint",
        ))?;

    let replay_started = Instant::now();
    let mut expected_state_hash = None;
    let mut expected_receipt_hash = None;
    for _ in 0..config.iterations {
        let mut ledger = empty_ledger(chain.clone())?;
        let request = replay_request(
            &chain,
            START_HEIGHT,
            end_height,
            manifests.clone(),
            ledger.state_hash(),
            schema_fingerprint,
        )?;
        let ReplayOutcome::Completed(receipt) =
            SerialReplayEngine::new(&archive, &mut ledger, ReplayLimits::production())
                .run(&request, &NeverCancel)?
        else {
            return Err(FixtureRunError::Invariant("account replay was cancelled"));
        };
        match (expected_state_hash, expected_receipt_hash) {
            (Some(state_hash), Some(receipt_hash))
                if state_hash == ledger.state_hash() && receipt_hash == receipt.receipt_hash() => {}
            (None, None) => {
                expected_state_hash = Some(ledger.state_hash());
                expected_receipt_hash = Some(receipt.receipt_hash());
            }
            _ => {
                return Err(FixtureRunError::Invariant(
                    "independent account replays diverged",
                ));
            }
        }
    }
    let replay_elapsed_micros =
        u64::try_from(replay_started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let expected_state_hash =
        expected_state_hash.ok_or(FixtureRunError::Invariant("missing replay state"))?;
    let expected_receipt_hash =
        expected_receipt_hash.ok_or(FixtureRunError::Invariant("missing replay receipt"))?;

    let checkpoint_len =
        usize::try_from(config.checkpoint_after).map_err(|_| FixtureRunError::InvalidConfig)?;
    let checkpoint_end = START_HEIGHT + config.checkpoint_after - 1;
    let mut partial = empty_ledger(chain.clone())?;
    let request = replay_request(
        &chain,
        START_HEIGHT,
        checkpoint_end,
        manifests[..checkpoint_len].to_vec(),
        partial.state_hash(),
        schema_fingerprint,
    )?;
    let ReplayOutcome::Completed(_) =
        SerialReplayEngine::new(&archive, &mut partial, ReplayLimits::production())
            .run(&request, &NeverCancel)?
    else {
        return Err(FixtureRunError::Invariant(
            "account checkpoint replay was cancelled",
        ));
    };
    let checkpoint_manifest = archive.verify_manifest(&manifests[checkpoint_len - 1])?;
    let artifact = CheckpointArtifact::try_new(
        partial
            .checkpoint()
            .ok_or(FixtureRunError::Invariant("checkpoint watermark is absent"))?,
        partial.state_image().clone(),
        checkpoint_manifest.manifest_id().clone(),
        checkpoint_manifest.manifest_sha256(),
        schema_fingerprint,
    )?;
    let store =
        LocalCheckpointStore::open(output.join("checkpoints"), StateImageLimits::production())?;
    let published = store.publish(&artifact)?;
    let compatibility = CheckpointCompatibility::try_new(
        chain.clone(),
        artifact.checkpoint().reducer_set_version(),
        artifact.archive_manifest_id().clone(),
        artifact.archive_manifest_sha256(),
        artifact.schema_fingerprint(),
    )?;
    let loaded = store.load(
        published.receipt().checkpoint_id(),
        &compatibility,
        StateImageLimits::production(),
    )?;
    let mut resumed = CanonicalLedger::try_from_state_image(
        loaded.state_image().clone(),
        composite_reducer()?,
        LedgerLimits::production(),
    )?;
    let resume_start = checkpoint_end + 1;
    let request = replay_request(
        &chain,
        resume_start,
        end_height,
        manifests[checkpoint_len..].to_vec(),
        resumed.state_hash(),
        schema_fingerprint,
    )?;
    let ReplayOutcome::Completed(resume_receipt) =
        SerialReplayEngine::new(&archive, &mut resumed, ReplayLimits::production())
            .run(&request, &NeverCancel)?
    else {
        return Err(FixtureRunError::Invariant(
            "account checkpoint resume was cancelled",
        ));
    };
    if resumed.state_hash() != expected_state_hash {
        return Err(FixtureRunError::Invariant(
            "account checkpoint suffix diverged",
        ));
    }
    let counts = namespace_counts(&resumed);
    if counts != AccountNamespaceCounts::expected() {
        return Err(FixtureRunError::Invariant(
            "account namespace counts diverged",
        ));
    }
    validate_account_records(&resumed)?;

    let rejection_height = end_height + 1;
    let missing_asset_archive = rejection_archive(&output, "missing-asset")?;
    let missing_asset = rejection(
        &missing_asset_archive,
        &chain,
        &resumed,
        rejection_height,
        missing_asset_block(rejection_height, &chain)?,
        schema_fingerprint,
    )?;
    let missing_market_archive = rejection_archive(&output, "missing-market")?;
    let missing_market = rejection(
        &missing_market_archive,
        &chain,
        &resumed,
        rejection_height,
        missing_market_block(rejection_height, &chain)?,
        schema_fingerprint,
    )?;
    let cross_component_archive = rejection_archive(&output, "cross-component")?;
    let cross_component = rejection(
        &cross_component_archive,
        &chain,
        &resumed,
        rejection_height,
        cross_component_block(rejection_height, &chain)?,
        schema_fingerprint,
    )?;
    let unsupported_archive = rejection_archive(&output, "unsupported-schema")?;
    let unsupported = rejection(
        &unsupported_archive,
        &chain,
        &resumed,
        rejection_height,
        account_flow_block(rejection_height, &chain, "1.1.0")?,
        schema_fingerprint,
    )?;

    let report = AccountReport {
        schema_version: ACCOUNT_REPORT_SCHEMA,
        evidence_class: ACCOUNT_EVIDENCE_CLASS,
        state_semantics: "exact_observed_account_flows_relations_and_modes",
        source_qualification: "synthetic_unassessed",
        reducer_version: CanonicalStateReducerV1::VERSION,
        synthetic_account_flow_contract_proven: true,
        position_state_qualified: false,
        episode_state_qualified: false,
        liquidation_state_qualified: false,
        settlement_state_qualified: false,
        funding_attribution_qualified: false,
        stage_1_qualified: false,
        stage_2_qualified: false,
        deployed_source_qualified: false,
        live_source_qualified: false,
        authoritative_opening_balance_qualified: false,
        venue_balance_reconciliation_qualified: false,
        twap_position_completeness_qualified: false,
        backstop_cost_basis_qualified: false,
        standard_margin_qualified: false,
        unified_margin_qualified: false,
        portfolio_margin_qualified: false,
        liquidation_price_qualified: false,
        book_state_qualified: false,
        signal_state_qualified: false,
        execution_qualified: false,
        block_count: config.blocks,
        checkpoint_after: config.checkpoint_after,
        iterations_completed: config.iterations,
        expected_final_state_hash: hex::encode(expected_state_hash),
        deterministic_replay_receipt_hash: hex::encode(expected_receipt_hash),
        checkpoint_id: artifact.checkpoint_id().as_str(),
        resumed_final_state_hash: hex::encode(resumed.state_hash()),
        resume_receipt_hash: hex::encode(resume_receipt.receipt_hash()),
        replay_elapsed_micros,
        account_fact_count: counts.account_fact_count,
        account_quantity_flow_current_count: counts.account_quantity_flow_current_count,
        account_quote_flow_current_count: counts.account_quote_flow_current_count,
        vault_principal_flow_current_count: counts.vault_principal_flow_current_count,
        vault_share_flow_current_count: counts.vault_share_flow_current_count,
        subaccount_master_current_count: counts.subaccount_master_current_count,
        account_vault_relation_current_count: counts.account_vault_relation_current_count,
        account_mode_current_count: counts.account_mode_current_count,
        margin_mode_current_count: counts.margin_mode_current_count,
        leverage_current_count: counts.leverage_current_count,
        missing_asset_prerequisite: missing_asset,
        missing_market_prerequisite: missing_market,
        cross_component_late_invalid: cross_component,
        unsupported_schema: unsupported,
    };
    let report_path = output.join(REPORT_FILE);
    publish_report(&report_path, &report)?;
    harden_private_tree(&output)?;
    Ok(AccountEvidence { report_path })
}

fn empty_ledger(
    chain: ChainId,
) -> Result<CanonicalLedger<CanonicalStateReducerV1>, FixtureRunError> {
    Ok(CanonicalLedger::try_new(
        chain,
        BlockHeight::new(START_HEIGHT),
        composite_reducer()?,
        LedgerLimits::production(),
    )?)
}

fn rejection(
    archive: &LocalParquetArchive,
    chain: &ChainId,
    source: &CanonicalLedger<CanonicalStateReducerV1>,
    height: u64,
    rejected: BlockEnvelope,
    schema_fingerprint: [u8; 32],
) -> Result<RejectionReport, FixtureRunError> {
    let manifest = archive.append_block(&rejected)?.manifest_id().clone();
    let mut ledger = CanonicalLedger::try_from_state_image(
        source.state_image().clone(),
        composite_reducer()?,
        LedgerLimits::production(),
    )?;
    let before = ledger.state_hash();
    let request = replay_request(
        chain,
        height,
        height,
        vec![manifest],
        before,
        schema_fingerprint,
    )?;
    let error = SerialReplayEngine::new(archive, &mut ledger, ReplayLimits::production())
        .run(&request, &NeverCancel)
        .expect_err("rejected account block must quarantine");
    let after = ledger.state_hash();
    let expected = if error.reason_code() == "replay.block_quarantined" {
        error.source_reason_code().unwrap_or("")
    } else {
        ""
    };
    if expected.is_empty() || after != before || error.progress().applied_block_count() != 0 {
        return Err(FixtureRunError::Invariant(
            "account rejection was not atomic",
        ));
    }
    rejection_report(
        height,
        &error,
        error.reducer_reason_code().map(str::to_owned),
        before,
        after,
    )
}

#[derive(Debug, PartialEq, Eq)]
struct AccountNamespaceCounts {
    account_fact_count: usize,
    account_quantity_flow_current_count: usize,
    account_quote_flow_current_count: usize,
    vault_principal_flow_current_count: usize,
    vault_share_flow_current_count: usize,
    subaccount_master_current_count: usize,
    account_vault_relation_current_count: usize,
    account_mode_current_count: usize,
    margin_mode_current_count: usize,
    leverage_current_count: usize,
}

impl AccountNamespaceCounts {
    const fn expected() -> Self {
        Self {
            account_fact_count: 15,
            account_quantity_flow_current_count: 10,
            account_quote_flow_current_count: 4,
            vault_principal_flow_current_count: 1,
            vault_share_flow_current_count: 1,
            subaccount_master_current_count: 1,
            account_vault_relation_current_count: 1,
            account_mode_current_count: 1,
            margin_mode_current_count: 1,
            leverage_current_count: 1,
        }
    }
}

fn namespace_counts(ledger: &CanonicalLedger<CanonicalStateReducerV1>) -> AccountNamespaceCounts {
    let count = |namespace| {
        ledger
            .state_image()
            .entries()
            .keys()
            .filter(|key| key.namespace() == namespace)
            .count()
    };
    AccountNamespaceCounts {
        account_fact_count: count("account-fact.v1"),
        account_quantity_flow_current_count: count("account-quantity-flow-current.v1"),
        account_quote_flow_current_count: count("account-quote-flow-current.v1"),
        vault_principal_flow_current_count: count("vault-principal-flow-current.v1"),
        vault_share_flow_current_count: count("vault-share-flow-current.v1"),
        subaccount_master_current_count: count("account-subaccount-master.v1"),
        account_vault_relation_current_count: count("account-vault-relation.v1"),
        account_mode_current_count: count("account-mode-current.v1"),
        margin_mode_current_count: count("account-margin-mode-current.v1"),
        leverage_current_count: count("account-leverage-current.v1"),
    }
}

fn validate_account_records(
    ledger: &CanonicalLedger<CanonicalStateReducerV1>,
) -> Result<(), FixtureRunError> {
    let usdc = AssetId::new("USDC")?;
    let market = market()?;
    let vault = VaultId::new("state-replay-vault")?;

    for (key, bytes) in ledger.state_image().entries() {
        let valid = match key.namespace() {
            "account-fact.v1" => AccountFactRecordV1::decode_at(key, bytes).is_ok(),
            "account-quantity-flow-current.v1" => {
                AccountQuantityFlowCurrentRecordV1::decode_at(key, bytes).is_ok()
            }
            "account-quote-flow-current.v1" => {
                AccountQuoteFlowCurrentRecordV1::decode_at(key, bytes).is_ok()
            }
            "vault-principal-flow-current.v1" => {
                VaultPrincipalFlowCurrentRecordV1::decode_at(key, bytes).is_ok()
            }
            "vault-share-flow-current.v1" => {
                VaultShareFlowCurrentRecordV1::decode_at(key, bytes).is_ok()
            }
            "account-subaccount-master.v1" => {
                SubaccountMasterCurrentRecordV1::decode_at(key, bytes).is_ok()
            }
            "account-vault-relation.v1" => {
                AccountVaultRelationCurrentRecordV1::decode_at(key, bytes).is_ok()
            }
            "account-mode-current.v1" => AccountModeCurrentRecordV1::decode_at(key, bytes).is_ok(),
            "account-margin-mode-current.v1" => {
                MarginModeCurrentRecordV1::decode_at(key, bytes).is_ok()
            }
            "account-leverage-current.v1" => LeverageCurrentRecordV1::decode_at(key, bytes).is_ok(),
            "asset-context-current.v1"
            | "dex-current.v1"
            | "market-current.v1"
            | "market-fact.v1"
            | "market-metadata-version.v1"
            | "market-outcome-current.v1"
            | "order-current.v1"
            | "order-fact.v1"
            | "order-transition.v1"
            | "position-episode-current.v1"
            | "position-episode-effect-fact.v1"
            | "position-episode.v1"
            | "position-effect-fact.v1"
            | "position-quantity-current.v1"
            | "position-unresolved-cause-fact.v1"
            | "position-settlement-fact.v1"
            | "reconciliation.v1"
            | "trade-participant.v1"
            | "trade-participant.v2"
            | "trade-reconciliation.v2"
            | "trade.v1"
            | "trade.v2" => true,
            _ => false,
        };
        if !valid {
            return Err(FixtureRunError::Invariant(
                "account evidence contains an unknown or unbound record namespace",
            ));
        }
    }

    let quantity = |account, scope, credits: &str, debits: &str| -> Result<(), FixtureRunError> {
        let key = account_key(AccountQuantityFlowCurrentRecordV1::state_key(
            &account, &scope,
        ))?;
        let record = ledger
            .state_image()
            .entries()
            .get(&key)
            .and_then(|bytes| AccountQuantityFlowCurrentRecordV1::decode_at(&key, bytes).ok())
            .ok_or(FixtureRunError::Invariant(
                "missing expected quantity-flow account leg",
            ))?;
        if !quantity_leg_matches(&record, account, &scope, credits, debits)? {
            return Err(FixtureRunError::Invariant(
                "quantity-flow account leg differs from fixture",
            ));
        }
        Ok(())
    };
    quantity(
        BUYER,
        AccountQuantityFlowScopeV1::ExternalAsset {
            asset_id: usdc.clone(),
        },
        "10",
        "2",
    )?;
    quantity(
        BUYER,
        AccountQuantityFlowScopeV1::SpotTransferAsset {
            asset_id: usdc.clone(),
        },
        "0",
        "1",
    )?;
    quantity(
        SELLER,
        AccountQuantityFlowScopeV1::SpotTransferAsset {
            asset_id: usdc.clone(),
        },
        "1",
        "0",
    )?;
    quantity(
        BUYER,
        AccountQuantityFlowScopeV1::SubaccountTransferAsset {
            asset_id: usdc.clone(),
        },
        "0",
        "1.5",
    )?;
    quantity(
        SELLER,
        AccountQuantityFlowScopeV1::SubaccountTransferAsset {
            asset_id: usdc.clone(),
        },
        "1.5",
        "0",
    )?;
    quantity(
        BUYER,
        AccountQuantityFlowScopeV1::VaultShares {
            vault_id: vault.clone(),
        },
        "2",
        "0.5",
    )?;
    quantity(
        BUYER,
        AccountQuantityFlowScopeV1::FeeAsset {
            asset_id: usdc.clone(),
        },
        "0",
        "0.1",
    )?;
    quantity(
        BUYER,
        AccountQuantityFlowScopeV1::BuilderFeeAsset {
            asset_id: usdc.clone(),
        },
        "0",
        "0.1",
    )?;
    quantity(
        BUILDER,
        AccountQuantityFlowScopeV1::BuilderFeeAsset {
            asset_id: usdc.clone(),
        },
        "0.1",
        "0",
    )?;
    quantity(
        REFERRER,
        AccountQuantityFlowScopeV1::ReferralRewardAsset {
            asset_id: usdc.clone(),
        },
        "0.1",
        "0",
    )?;

    let quote = |account, scope, credits: &str, debits: &str| -> Result<(), FixtureRunError> {
        let key = account_key(AccountQuoteFlowCurrentRecordV1::state_key(&account, &scope))?;
        let record = ledger
            .state_image()
            .entries()
            .get(&key)
            .and_then(|bytes| AccountQuoteFlowCurrentRecordV1::decode_at(&key, bytes).ok())
            .ok_or(FixtureRunError::Invariant(
                "missing expected quote-flow account leg",
            ))?;
        if record.account_id() != account
            || record.scope() != &scope
            || record.credits() != QuoteAmount::from_str(credits)?
            || record.debits() != QuoteAmount::from_str(debits)?
        {
            return Err(FixtureRunError::Invariant(
                "quote-flow account leg differs from fixture",
            ));
        }
        Ok(())
    };
    quote(BUYER, AccountQuoteFlowScopeV1::DefaultPerpQuote, "0", "3")?;
    quote(SELLER, AccountQuoteFlowScopeV1::DefaultPerpQuote, "3", "0")?;
    quote(
        BUYER,
        AccountQuoteFlowScopeV1::VaultPrincipal {
            vault_id: vault.clone(),
        },
        "1",
        "4",
    )?;
    quote(
        BUYER,
        AccountQuoteFlowScopeV1::MarketFunding {
            market_id: market.clone(),
        },
        "0.2",
        "0.4",
    )?;

    let principal_key = account_key(VaultPrincipalFlowCurrentRecordV1::state_key(&vault))?;
    let principal = ledger
        .state_image()
        .entries()
        .get(&principal_key)
        .and_then(|bytes| VaultPrincipalFlowCurrentRecordV1::decode_at(&principal_key, bytes).ok())
        .ok_or(FixtureRunError::Invariant(
            "missing expected vault principal leg",
        ))?;
    if principal.vault_id() != &vault
        || principal.deposits() != QuoteAmount::from_str("4")?
        || principal.withdrawals() != QuoteAmount::from_str("1")?
    {
        return Err(FixtureRunError::Invariant(
            "vault principal leg differs from fixture",
        ));
    }
    let shares_key = account_key(VaultShareFlowCurrentRecordV1::state_key(&vault))?;
    let shares = ledger
        .state_image()
        .entries()
        .get(&shares_key)
        .and_then(|bytes| VaultShareFlowCurrentRecordV1::decode_at(&shares_key, bytes).ok())
        .ok_or(FixtureRunError::Invariant(
            "missing expected vault share leg",
        ))?;
    if shares.vault_id() != &vault
        || shares.shares_issued() != Quantity::from_str("2")?
        || shares.shares_redeemed() != Quantity::from_str("0.5")?
    {
        return Err(FixtureRunError::Invariant(
            "vault share leg differs from fixture",
        ));
    }
    let subaccount_key = account_key(SubaccountMasterCurrentRecordV1::state_key(&SELLER))?;
    let subaccount = ledger
        .state_image()
        .entries()
        .get(&subaccount_key)
        .and_then(|bytes| SubaccountMasterCurrentRecordV1::decode_at(&subaccount_key, bytes).ok())
        .ok_or(FixtureRunError::Invariant(
            "missing expected subaccount relation",
        ))?;
    if subaccount.subaccount_id() != SELLER || subaccount.master_account_id() != BUYER {
        return Err(FixtureRunError::Invariant(
            "subaccount relation differs from fixture",
        ));
    }
    let relation_key = account_key(AccountVaultRelationCurrentRecordV1::state_key(
        &BUYER, &vault,
    ))?;
    let relation = ledger
        .state_image()
        .entries()
        .get(&relation_key)
        .and_then(|bytes| AccountVaultRelationCurrentRecordV1::decode_at(&relation_key, bytes).ok())
        .ok_or(FixtureRunError::Invariant(
            "missing expected vault relation",
        ))?;
    if relation.account_id() != BUYER || relation.vault_id() != &vault {
        return Err(FixtureRunError::Invariant(
            "vault relation differs from fixture",
        ));
    }
    let mode_key = account_key(AccountModeCurrentRecordV1::state_key(&BUYER))?;
    let mode = ledger
        .state_image()
        .entries()
        .get(&mode_key)
        .and_then(|bytes| AccountModeCurrentRecordV1::decode_at(&mode_key, bytes).ok())
        .ok_or(FixtureRunError::Invariant("missing expected account mode"))?;
    if mode.account_id() != BUYER
        || mode.initial_previous() != AccountAbstractionModeV1::Standard
        || mode.current() != AccountAbstractionModeV1::Unified
    {
        return Err(FixtureRunError::Invariant(
            "account mode differs from fixture",
        ));
    }
    let margin_key = account_key(MarginModeCurrentRecordV1::state_key(&BUYER, &market))?;
    let margin = ledger
        .state_image()
        .entries()
        .get(&margin_key)
        .and_then(|bytes| MarginModeCurrentRecordV1::decode_at(&margin_key, bytes).ok())
        .ok_or(FixtureRunError::Invariant("missing expected margin mode"))?;
    if margin.account_id() != BUYER
        || margin.market_id() != &market
        || margin.initial_previous() != MarginModeV1::Cross
        || margin.current() != MarginModeV1::Isolated
    {
        return Err(FixtureRunError::Invariant(
            "margin mode differs from fixture",
        ));
    }
    let leverage_key = account_key(LeverageCurrentRecordV1::state_key(&BUYER, &market))?;
    let leverage = ledger
        .state_image()
        .entries()
        .get(&leverage_key)
        .and_then(|bytes| LeverageCurrentRecordV1::decode_at(&leverage_key, bytes).ok())
        .ok_or(FixtureRunError::Invariant("missing expected leverage"))?;
    if leverage.account_id() != BUYER
        || leverage.market_id() != &market
        || leverage.initial_previous() != Leverage::from_str("1")?
        || leverage.current() != Leverage::from_str("5")?
    {
        return Err(FixtureRunError::Invariant("leverage differs from fixture"));
    }
    let mut expected_facts = vec![
        (
            EventKind::DepositCredited,
            vec![BUYER],
            vec![],
            Some(usdc.clone()),
            None,
        ),
        (
            EventKind::WithdrawalDebited,
            vec![BUYER],
            vec![],
            Some(usdc.clone()),
            None,
        ),
        (
            EventKind::SpotTransfer,
            vec![BUYER, SELLER],
            vec![],
            Some(usdc.clone()),
            None,
        ),
        (
            EventKind::PerpTransfer,
            vec![BUYER, SELLER],
            vec![],
            None,
            None,
        ),
        (
            EventKind::SubaccountTransfer,
            vec![BUYER, BUYER, SELLER],
            vec![],
            Some(usdc.clone()),
            None,
        ),
        (
            EventKind::VaultDeposit,
            vec![BUYER],
            vec![],
            None,
            Some(vault.clone()),
        ),
        (
            EventKind::VaultWithdrawal,
            vec![BUYER],
            vec![],
            None,
            Some(vault.clone()),
        ),
        (
            EventKind::FeeCharged,
            vec![BUYER],
            vec![],
            Some(usdc.clone()),
            None,
        ),
        (
            EventKind::BuilderFeeCharged,
            vec![BUYER, BUILDER],
            vec![],
            Some(usdc.clone()),
            None,
        ),
        (
            EventKind::ReferralReward,
            vec![BUYER, REFERRER],
            vec![],
            Some(usdc.clone()),
            None,
        ),
        (
            EventKind::AccountModeChanged,
            vec![BUYER],
            vec![],
            None,
            None,
        ),
        (
            EventKind::MarginModeChanged,
            vec![BUYER],
            vec![market.clone()],
            None,
            None,
        ),
        (
            EventKind::LeverageChanged,
            vec![BUYER],
            vec![market.clone()],
            None,
            None,
        ),
        (
            EventKind::FundingPaid,
            vec![BUYER],
            vec![market.clone()],
            None,
            None,
        ),
        (
            EventKind::FundingReceived,
            vec![BUYER],
            vec![market],
            None,
            None,
        ),
    ];
    for (key, bytes) in ledger
        .state_image()
        .entries()
        .iter()
        .filter(|(key, _)| key.namespace() == "account-fact.v1")
    {
        let fact = AccountFactRecordV1::decode_at(key, bytes)
            .map_err(|_| FixtureRunError::Invariant("account fact is not key-bound"))?;
        let position = expected_facts.iter().position(|expected| {
            fact.event_kind() == expected.0
                && fact.account_ids() == expected.1.as_slice()
                && fact.market_ids() == expected.2.as_slice()
                && fact.asset_id() == expected.3.as_ref()
                && fact.vault_id() == expected.4.as_ref()
        });
        let position = position.ok_or(FixtureRunError::Invariant(
            "account fact identity differs from literal fixture",
        ))?;
        expected_facts.swap_remove(position);
    }
    if !expected_facts.is_empty() {
        return Err(FixtureRunError::Invariant(
            "expected account facts are absent",
        ));
    }
    Ok(())
}

fn account_key<T>(result: Result<T, AccountStateError>) -> Result<T, FixtureRunError> {
    result.map_err(|_| FixtureRunError::Invariant("constructing expected account state key failed"))
}

fn quantity_leg_matches(
    record: &AccountQuantityFlowCurrentRecordV1,
    account: Address,
    scope: &AccountQuantityFlowScopeV1,
    credits: &str,
    debits: &str,
) -> Result<bool, FixtureRunError> {
    Ok(record.account_id() == account
        && record.scope() == scope
        && record.credits() == Quantity::from_str(credits)?
        && record.debits() == Quantity::from_str(debits)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_quantity_validator_rejects_a_key_bound_reversed_flow() {
        let scope = AccountQuantityFlowScopeV1::SpotTransferAsset {
            asset_id: AssetId::new("USDC").expect("fixture asset"),
        };
        let key =
            AccountQuantityFlowCurrentRecordV1::state_key(&SELLER, &scope).expect("fixture key");
        let bytes = format!(
            concat!(
                r#"{{"schema":"hyperliquid-alpha-desk/account-quantity-flow-current/v1","#,
                r#""account_id":"{}","scope":"spot_transfer_asset","#,
                r#""asset_id":"USDC","vault_id":null,"credits":"0","#,
                r#""debits":"1","last_event_id":"fixture-reversed-flow","#,
                r#""last_block_height":1}}"#,
            ),
            SELLER.to_api_string(),
        )
        .into_bytes();
        let record = AccountQuantityFlowCurrentRecordV1::decode_at(&key, &bytes)
            .expect("well-formed key-bound mutated record");

        assert!(
            !quantity_leg_matches(&record, SELLER, &scope, "1", "0").expect("literal comparison")
        );
    }
}

fn market_prerequisite_block(
    height: u64,
    chain: &ChainId,
) -> Result<BlockEnvelope, FixtureRunError> {
    block(height, chain, market_prerequisite_events(height)?)
}

pub(super) fn market_prerequisite_events(
    height: u64,
) -> Result<Vec<CanonicalEventEnvelope>, FixtureRunError> {
    let btc = AssetId::new("BTC")?;
    let usdc = AssetId::new("USDC")?;
    let market = market()?;
    Ok(vec![
        event(
            height,
            0,
            EventPayload::DexCreated(DexCreated {
                dex_id: DexId::new("state-replay")?,
                name: "State replay".to_owned(),
                operator_account_id: OPERATOR,
            }),
            vec![],
            vec![OPERATOR],
            "1.0.0",
        )?,
        event(
            height,
            1,
            EventPayload::AssetContextUpdated(AssetContextUpdated {
                asset_id: btc.clone(),
                context_version: "btc-v1".to_owned(),
                context_hash: [1; 32],
            }),
            vec![],
            vec![],
            "1.0.0",
        )?,
        event(
            height,
            2,
            EventPayload::AssetContextUpdated(AssetContextUpdated {
                asset_id: usdc.clone(),
                context_version: "usdc-v1".to_owned(),
                context_hash: [2; 32],
            }),
            vec![],
            vec![],
            "1.0.0",
        )?,
        event(
            height,
            3,
            EventPayload::MarketCreated(MarketCreated {
                market_id: market.clone(),
                dex_id: DexId::new("state-replay")?,
                base_asset_id: btc,
                quote_asset_id: usdc,
                tick_size: Price::parse_at_scale("0.1", 6)?,
                lot_size: Quantity::parse_at_scale("0.001", 8)?,
            }),
            vec![market],
            vec![],
            "1.0.0",
        )?,
    ])
}

fn account_flow_block(
    height: u64,
    chain: &ChainId,
    schema: &str,
) -> Result<BlockEnvelope, FixtureRunError> {
    let usdc = AssetId::new("USDC")?;
    let market = market()?;
    let vault = VaultId::new("state-replay-vault")?;
    let mut events = vec![
        event(
            height,
            0,
            EventPayload::DepositCredited(DepositCredited {
                account_id: BUYER,
                asset_id: usdc.clone(),
                amount: Quantity::from_str("10")?,
                deposit_reference: "deposit".to_owned(),
            }),
            vec![],
            vec![BUYER],
            schema,
        )?,
        event(
            height,
            1,
            EventPayload::WithdrawalDebited(WithdrawalDebited {
                account_id: BUYER,
                asset_id: usdc.clone(),
                amount: Quantity::from_str("2")?,
                withdrawal_reference: "withdrawal".to_owned(),
            }),
            vec![],
            vec![BUYER],
            schema,
        )?,
        event(
            height,
            2,
            EventPayload::SpotTransfer(SpotTransfer {
                from_account_id: BUYER,
                to_account_id: SELLER,
                asset_id: usdc.clone(),
                amount: Quantity::from_str("1")?,
            }),
            vec![],
            vec![BUYER, SELLER],
            schema,
        )?,
        event(
            height,
            3,
            EventPayload::PerpTransfer(PerpTransfer {
                from_account_id: BUYER,
                to_account_id: SELLER,
                quote_amount: QuoteAmount::from_str("3")?,
            }),
            vec![],
            vec![BUYER, SELLER],
            schema,
        )?,
        event(
            height,
            4,
            EventPayload::SubaccountTransfer(SubaccountTransfer {
                master_account_id: BUYER,
                from_account_id: BUYER,
                to_account_id: SELLER,
                asset_id: usdc.clone(),
                amount: Quantity::from_str("1.5")?,
            }),
            vec![],
            vec![BUYER, BUYER, SELLER],
            schema,
        )?,
        event(
            height,
            5,
            EventPayload::VaultDeposit(VaultDeposit {
                vault_id: vault.clone(),
                account_id: BUYER,
                amount: QuoteAmount::from_str("4")?,
                shares_issued: Quantity::from_str("2")?,
            }),
            vec![],
            vec![BUYER],
            schema,
        )?,
        event(
            height,
            6,
            EventPayload::VaultWithdrawal(VaultWithdrawal {
                vault_id: vault.clone(),
                account_id: BUYER,
                amount: QuoteAmount::from_str("1")?,
                shares_redeemed: Quantity::from_str("0.5")?,
            }),
            vec![],
            vec![BUYER],
            schema,
        )?,
        event(
            height,
            7,
            EventPayload::FeeCharged(FeeCharged {
                account_id: BUYER,
                asset_id: usdc.clone(),
                amount: Quantity::from_str("0.1")?,
                fee_rate: FeeRate::from_str("0.001")?,
                fee_type: FeeTypeV1::Taker,
            }),
            vec![],
            vec![BUYER],
            schema,
        )?,
        event(
            height,
            8,
            EventPayload::BuilderFeeCharged(BuilderFeeCharged {
                account_id: BUYER,
                builder_account_id: BUILDER,
                asset_id: usdc.clone(),
                amount: Quantity::from_str("0.1")?,
            }),
            vec![],
            vec![BUYER, BUILDER],
            schema,
        )?,
        event(
            height,
            9,
            EventPayload::ReferralReward(ReferralReward {
                account_id: BUYER,
                referrer_account_id: REFERRER,
                asset_id: usdc,
                amount: Quantity::from_str("0.1")?,
            }),
            vec![],
            vec![BUYER, REFERRER],
            schema,
        )?,
        event(
            height,
            10,
            EventPayload::AccountModeChanged(AccountModeChanged {
                account_id: BUYER,
                previous_mode: AccountAbstractionModeV1::Standard,
                new_mode: AccountAbstractionModeV1::Unified,
            }),
            vec![],
            vec![BUYER],
            schema,
        )?,
        event(
            height,
            11,
            EventPayload::MarginModeChanged(MarginModeChanged {
                account_id: BUYER,
                market_id: market.clone(),
                previous_mode: MarginModeV1::Cross,
                new_mode: MarginModeV1::Isolated,
            }),
            vec![market.clone()],
            vec![BUYER],
            schema,
        )?,
        event(
            height,
            12,
            EventPayload::LeverageChanged(canonical_events::LeverageChanged {
                account_id: BUYER,
                market_id: market.clone(),
                previous_leverage: Leverage::from_str("1")?,
                new_leverage: Leverage::from_str("5")?,
            }),
            vec![market.clone()],
            vec![BUYER],
            schema,
        )?,
        event(
            height,
            13,
            EventPayload::OrderAccepted(OrderAccepted {
                order_id: OrderId::new("account-buyer-order")?,
                account_id: BUYER,
                market_id: market.clone(),
                side: OrderSide::Buy,
                limit_price: Price::parse_at_scale("65000", 6)?,
                quantity: Quantity::parse_at_scale("1", 8)?,
            }),
            vec![market.clone()],
            vec![BUYER],
            schema,
        )?,
        event(
            height,
            14,
            EventPayload::OrderAccepted(OrderAccepted {
                order_id: OrderId::new("account-seller-order")?,
                account_id: SELLER,
                market_id: market.clone(),
                side: OrderSide::Sell,
                limit_price: Price::parse_at_scale("65000", 6)?,
                quantity: Quantity::parse_at_scale("1", 8)?,
            }),
            vec![market.clone()],
            vec![SELLER],
            schema,
        )?,
        event(
            height,
            15,
            EventPayload::TradeMatched(TradeMatched {
                trade_id: Some(TradeId::new("account-funding-anchor")?),
                market_id: Some(market.clone()),
                maker_order_id: Some(OrderId::new("account-seller-order")?),
                taker_order_id: Some(OrderId::new("account-buyer-order")?),
                price: Price::parse_at_scale("65000", 6)?,
                quantity: Quantity::parse_at_scale("0.25", 8)?,
                deterministic_seed: height,
                participants: Some(Box::new([
                    TradeParticipantV1 {
                        role: TradeParticipantRoleV1::Buyer,
                        account_id: BUYER,
                        start_position: PositionQuantity::from_str("0")?,
                        order_id: OrderId::new("account-buyer-order")?,
                        twap_id: None,
                        client_order_id: None,
                    },
                    TradeParticipantV1 {
                        role: TradeParticipantRoleV1::Seller,
                        account_id: SELLER,
                        start_position: PositionQuantity::from_str("0")?,
                        order_id: OrderId::new("account-seller-order")?,
                        twap_id: None,
                        client_order_id: None,
                    },
                ])),
            }),
            vec![market.clone()],
            vec![BUYER, SELLER],
            schema,
        )?,
        event(
            height,
            16,
            EventPayload::FundingPaid(FundingPaid {
                account_id: BUYER,
                market_id: market.clone(),
                amount: QuoteAmount::from_str("0.4")?,
                funding_rate: FundingRate::from_str("0.0001")?,
            }),
            vec![market.clone()],
            vec![BUYER],
            schema,
        )?,
        event(
            height,
            17,
            EventPayload::FundingReceived(FundingReceived {
                account_id: BUYER,
                market_id: market.clone(),
                amount: QuoteAmount::from_str("0.2")?,
                funding_rate: FundingRate::from_str("0.0001")?,
            }),
            vec![market],
            vec![BUYER],
            schema,
        )?,
    ];
    events.sort_by_key(|event| event.canonical_event_index());
    block(height, chain, events)
}

fn missing_asset_block(height: u64, chain: &ChainId) -> Result<BlockEnvelope, FixtureRunError> {
    block(
        height,
        chain,
        vec![event(
            height,
            0,
            EventPayload::DepositCredited(DepositCredited {
                account_id: BUYER,
                asset_id: AssetId::new("ETH")?,
                amount: Quantity::from_str("1")?,
                deposit_reference: "missing-asset".to_owned(),
            }),
            vec![],
            vec![BUYER],
            "1.0.0",
        )?],
    )
}

fn missing_market_block(height: u64, chain: &ChainId) -> Result<BlockEnvelope, FixtureRunError> {
    let missing = MarketId::new("perp:ETH")?;
    block(
        height,
        chain,
        vec![event(
            height,
            0,
            EventPayload::MarginModeChanged(MarginModeChanged {
                account_id: BUYER,
                market_id: missing.clone(),
                previous_mode: MarginModeV1::Cross,
                new_mode: MarginModeV1::Isolated,
            }),
            vec![missing],
            vec![BUYER],
            "1.0.0",
        )?],
    )
}

fn cross_component_block(height: u64, chain: &ChainId) -> Result<BlockEnvelope, FixtureRunError> {
    let market = market()?;
    block(
        height,
        chain,
        vec![
            event(
                height,
                0,
                EventPayload::DepositCredited(DepositCredited {
                    account_id: BUYER,
                    asset_id: AssetId::new("USDC")?,
                    amount: Quantity::from_str("1")?,
                    deposit_reference: "late-invalid-prefix".to_owned(),
                }),
                vec![],
                vec![BUYER],
                "1.0.0",
            )?,
            event(
                height,
                1,
                EventPayload::TradeMatched(TradeMatched {
                    trade_id: Some(TradeId::new("late-invalid-duplicate")?),
                    market_id: Some(market.clone()),
                    maker_order_id: None,
                    taker_order_id: None,
                    price: Price::parse_at_scale("65000", 6)?,
                    quantity: Quantity::parse_at_scale("0.01", 8)?,
                    deterministic_seed: height,
                    participants: None,
                }),
                vec![market.clone()],
                vec![BUYER, SELLER],
                "1.0.0",
            )?,
            event(
                height,
                2,
                EventPayload::TradeMatched(TradeMatched {
                    trade_id: Some(TradeId::new("late-invalid-duplicate")?),
                    market_id: Some(market.clone()),
                    maker_order_id: None,
                    taker_order_id: None,
                    price: Price::parse_at_scale("65000", 6)?,
                    quantity: Quantity::parse_at_scale("0.01", 8)?,
                    deterministic_seed: height,
                    participants: None,
                }),
                vec![market],
                vec![BUYER, SELLER],
                "1.0.0",
            )?,
        ],
    )
}

fn event(
    height: u64,
    index: u32,
    payload: EventPayload,
    market_ids: Vec<MarketId>,
    account_ids: Vec<Address>,
    schema: &str,
) -> Result<CanonicalEventEnvelope, FixtureRunError> {
    let time = fixture_time(height)?;
    let payload_hash = *blake3::hash(&payload.encode_to_vec()?).as_bytes();
    Ok(CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: schema.to_owned(),
        chain_id: ChainId::new(CHAIN)?,
        block_height: BlockHeight::new(height),
        block_time: time,
        transaction_id: TransactionId::new(format!("state-replay-account-{height}-{index}"))?,
        transaction_index: index,
        canonical_event_index: 0,
        market_ids,
        account_ids,
        source_evidence: vec![SourceEvidence::try_new_indexed(
            SourceId::new("state-replay-account")?,
            "v1",
            height.to_string(),
            payload_hash,
            index,
        )?],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(time.unix_micros())?,
        ingested_at: KnownTime::from_unix_micros(time.unix_micros())?,
        canonicalized_at: KnownTime::from_unix_micros(time.unix_micros())?,
        parser_version: "state-replay-account-fixture-v1".to_owned(),
        payload,
    })?)
}

pub(super) fn block(
    height: u64,
    chain: &ChainId,
    events: Vec<CanonicalEventEnvelope>,
) -> Result<BlockEnvelope, FixtureRunError> {
    Ok(BlockEnvelope::try_new(
        chain.clone(),
        BlockHeight::new(height),
        fixture_time(height)?,
        ConfirmationClass::CommittedPrimary,
        events,
        source_hashes(height)?,
    )?)
}

fn market() -> Result<MarketId, FixtureRunError> {
    Ok(MarketId::new("perp:BTC")?)
}

fn rejection_archive(
    output: &std::path::Path,
    name: &str,
) -> Result<LocalParquetArchive, FixtureRunError> {
    Ok(LocalParquetArchive::open(
        output.join(name),
        ArchiveConfig::deterministic_fixture(
            format!("state-replay-account-{name}-v1"),
            KnownTime::from_unix_micros(FIXTURE_EPOCH_MICROS)?,
        )?,
    )?)
}

pub(super) fn composite_reducer() -> Result<CanonicalStateReducerV1, FixtureRunError> {
    CanonicalStateReducerV1::try_new().map_err(|_| {
        FixtureRunError::Invariant("canonical composite reducer configuration is invalid")
    })
}

#[derive(Debug, Serialize)]
struct AccountReport<'a> {
    schema_version: &'a str,
    evidence_class: &'a str,
    state_semantics: &'a str,
    source_qualification: &'a str,
    reducer_version: &'a str,
    synthetic_account_flow_contract_proven: bool,
    position_state_qualified: bool,
    episode_state_qualified: bool,
    liquidation_state_qualified: bool,
    settlement_state_qualified: bool,
    funding_attribution_qualified: bool,
    stage_1_qualified: bool,
    stage_2_qualified: bool,
    deployed_source_qualified: bool,
    live_source_qualified: bool,
    authoritative_opening_balance_qualified: bool,
    venue_balance_reconciliation_qualified: bool,
    twap_position_completeness_qualified: bool,
    backstop_cost_basis_qualified: bool,
    standard_margin_qualified: bool,
    unified_margin_qualified: bool,
    portfolio_margin_qualified: bool,
    liquidation_price_qualified: bool,
    book_state_qualified: bool,
    signal_state_qualified: bool,
    execution_qualified: bool,
    block_count: u64,
    checkpoint_after: u64,
    iterations_completed: u64,
    expected_final_state_hash: String,
    deterministic_replay_receipt_hash: String,
    checkpoint_id: &'a str,
    resumed_final_state_hash: String,
    resume_receipt_hash: String,
    replay_elapsed_micros: u64,
    account_fact_count: usize,
    account_quantity_flow_current_count: usize,
    account_quote_flow_current_count: usize,
    vault_principal_flow_current_count: usize,
    vault_share_flow_current_count: usize,
    subaccount_master_current_count: usize,
    account_vault_relation_current_count: usize,
    account_mode_current_count: usize,
    margin_mode_current_count: usize,
    leverage_current_count: usize,
    missing_asset_prerequisite: RejectionReport,
    missing_market_prerequisite: RejectionReport,
    cross_component_late_invalid: RejectionReport,
    unsupported_schema: RejectionReport,
}
