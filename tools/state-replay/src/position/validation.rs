use super::*;

pub(super) fn validate_checkpoint_entries(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    expected: &FrozenExpectation,
) -> Result<(), FixtureRunError> {
    assert_quantity(
        entries,
        BUYER,
        &expected.market,
        Some("2.00000000"),
        &expected.opening_event,
    )?;
    assert_quantity(
        entries,
        SELLER,
        &expected.market,
        Some("-2.00000000"),
        &expected.opening_event,
    )?;
    assert_position_effect(
        entries,
        OPEN_TRADE,
        TradeParticipantRoleV1::Buyer,
        BUYER,
        "0.00000000",
        "2.00000000",
        PositionAnchorTransitionV1::FirstObservation,
    )?;
    assert_position_effect(
        entries,
        OPEN_TRADE,
        TradeParticipantRoleV1::Seller,
        SELLER,
        "0.00000000",
        "-2.00000000",
        PositionAnchorTransitionV1::FirstObservation,
    )?;
    assert_open_episode(
        entries,
        &expected.opening_buyer_episode,
        BUYER,
        &expected.opening_event,
        EpisodeCompletenessV1::CompleteFromFlat,
        &expected.opening_event,
        "2.00000000",
        "200",
        "0.00000000",
        "0",
        "0",
    )?;
    assert_open_episode(
        entries,
        &expected.opening_seller_episode,
        SELLER,
        &expected.opening_event,
        EpisodeCompletenessV1::CompleteFromFlat,
        &expected.opening_event,
        "0.00000000",
        "0",
        "2.00000000",
        "200",
        "0",
    )?;
    if namespace_count(entries, "position-quantity-current.v1") != 2
        || namespace_count(entries, "position-effect-fact.v1") != 2
        || namespace_count(entries, "position-episode-current.v1") != 2
        || namespace_count(entries, "position-episode.v1") != 2
        || namespace_count(entries, "position-episode-effect-fact.v1") != 2
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

pub(super) fn validate_interrupted_entries(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    expected: &FrozenExpectation,
) -> Result<(), FixtureRunError> {
    assert_quantity(
        entries,
        BUYER,
        &expected.market,
        None,
        &expected.backstop_event,
    )?;
    assert_quantity(
        entries,
        SELLER,
        &expected.market,
        None,
        &expected.backstop_event,
    )?;
    for account in [BUYER, SELLER] {
        let current_key = PositionEpisodeCurrentRecordV1::state_key(&account, &expected.market)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
        let current = decode_at(
            entries,
            &current_key,
            PositionEpisodeCurrentRecordV1::decode_at,
        )?;
        if current.account_id() != account
            || current.episode_id().is_some()
            || current.attribution_resolution() != EpisodeAttributionResolutionV1::Interrupted
            || current.last_event_id() != &expected.backstop_event
        {
            return Err(FixtureRunError::PositionSemanticMismatch);
        }
        let cause_key = PositionUnresolvedCauseFactRecordV1::state_key(
            &account,
            &expected.market,
            &expected.backstop_event,
            &expected.liquidation,
        )
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
        let cause = decode_at(
            entries,
            &cause_key,
            PositionUnresolvedCauseFactRecordV1::decode_at,
        )?;
        if cause.cause() != PositionUnresolvedCauseV1::BackstopLiquidation {
            return Err(FixtureRunError::PositionSemanticMismatch);
        }
    }
    assert_interrupted_episode(
        entries,
        &expected.liquidation_remainder_episode,
        BUYER,
        &expected.backstop_event,
        EpisodeCloseCauseV1::BackstopInterrupted,
    )?;
    assert_interrupted_episode(
        entries,
        &expected.reversal_buyer_episode,
        SELLER,
        &expected.backstop_event,
        EpisodeCloseCauseV1::BackstopInterrupted,
    )?;
    if entries.iter().any(|(key, bytes)| {
        (key.namespace() == "position-episode.v1"
            || key.namespace() == "position-episode-effect-fact.v1")
            && bytes
                .windows(expected.interrupted_funding_event.as_str().len())
                .any(|window| window == expected.interrupted_funding_event.as_str().as_bytes())
    }) {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

pub(super) fn validate_final_entries(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    expected: &FrozenExpectation,
) -> Result<(), FixtureRunError> {
    assert_quantity(
        entries,
        BUYER,
        &expected.market,
        Some("4.25000000"),
        &expected.recovery_event,
    )?;
    assert_quantity(
        entries,
        SELLER,
        &expected.market,
        Some("-0.25000000"),
        &expected.recovery_event,
    )?;
    assert_position_effect(
        entries,
        RECOVERY_TRADE,
        TradeParticipantRoleV1::Buyer,
        BUYER,
        "4.00000000",
        "4.25000000",
        PositionAnchorTransitionV1::ReanchoredFromUnresolved,
    )?;
    assert_position_effect(
        entries,
        RECOVERY_TRADE,
        TradeParticipantRoleV1::Seller,
        SELLER,
        "0.00000000",
        "-0.25000000",
        PositionAnchorTransitionV1::ReanchoredFromUnresolved,
    )?;
    assert_open_episode(
        entries,
        &expected.recovery_buyer_episode,
        BUYER,
        &expected.recovery_event,
        EpisodeCompletenessV1::PartialFromFirstObservation,
        &expected.recovered_funding_event,
        "0.25000000",
        "23.75",
        "0.00000000",
        "0",
        "0.75",
    )?;
    assert_open_episode(
        entries,
        &expected.recovery_seller_episode,
        SELLER,
        &expected.recovery_event,
        EpisodeCompletenessV1::CompleteFromFlat,
        &expected.recovery_event,
        "0.00000000",
        "0",
        "0.25000000",
        "23.75",
        "0",
    )?;
    assert_episode_snapshot(
        entries,
        &expected.opening_buyer_episode,
        BUYER,
        &expected.opening_event,
        0,
        "0.00000000",
        EpisodeCompletenessV1::CompleteFromFlat,
        "2.00000000",
        "200",
        "2.00000000",
        "220",
        "0",
        EpisodeStatusV1::Closed,
        &expected.reversal_event,
        EpisodeCloseCauseV1::TradeReversal,
    )?;
    assert_episode_snapshot(
        entries,
        &expected.opening_seller_episode,
        SELLER,
        &expected.opening_event,
        0,
        "0.00000000",
        EpisodeCompletenessV1::CompleteFromFlat,
        "2.00000000",
        "220",
        "2.00000000",
        "200",
        "0",
        EpisodeStatusV1::Closed,
        &expected.reversal_event,
        EpisodeCloseCauseV1::TradeReversal,
    )?;
    assert_episode_snapshot(
        entries,
        &expected.reversal_seller_episode,
        BUYER,
        &expected.reversal_event,
        1,
        "0.00000000",
        EpisodeCompletenessV1::CompleteFromFlat,
        "0.00000000",
        "0",
        "1.00000000",
        "110",
        "1.25",
        EpisodeStatusV1::Interrupted,
        &expected.liquidation_fill_event,
        EpisodeCloseCauseV1::LiquidationFill,
    )?;
    assert_episode_snapshot(
        entries,
        &expected.reversal_buyer_episode,
        SELLER,
        &expected.reversal_event,
        1,
        "0.00000000",
        EpisodeCompletenessV1::CompleteFromFlat,
        "1.00000000",
        "110",
        "0.00000000",
        "0",
        "0",
        EpisodeStatusV1::Interrupted,
        &expected.backstop_event,
        EpisodeCloseCauseV1::BackstopInterrupted,
    )?;
    assert_episode_snapshot(
        entries,
        &expected.liquidation_remainder_episode,
        BUYER,
        &expected.liquidation_fill_event,
        1,
        "-0.75000000",
        EpisodeCompletenessV1::PartialFromFirstObservation,
        "0.00000000",
        "0",
        "0.00000000",
        "0",
        "0",
        EpisodeStatusV1::Interrupted,
        &expected.backstop_event,
        EpisodeCloseCauseV1::BackstopInterrupted,
    )?;
    assert_episode_effect(
        entries,
        &expected.opening_event,
        BUYER,
        0,
        &expected.opening_buyer_episode,
        EpisodeEffectKindV1::Opened,
        "2.00000000",
        "200",
        "0.00000000",
        "0",
        "0",
        None,
    )?;
    assert_episode_effect(
        entries,
        &expected.opening_event,
        SELLER,
        0,
        &expected.opening_seller_episode,
        EpisodeEffectKindV1::Opened,
        "0.00000000",
        "0",
        "2.00000000",
        "200",
        "0",
        None,
    )?;
    assert_episode_effect(
        entries,
        &expected.reversal_event,
        BUYER,
        0,
        &expected.opening_buyer_episode,
        EpisodeEffectKindV1::Closed,
        "0.00000000",
        "0",
        "2.00000000",
        "220",
        "0",
        Some(EpisodeCloseCauseV1::TradeReversal),
    )?;
    assert_episode_effect(
        entries,
        &expected.reversal_event,
        BUYER,
        1,
        &expected.reversal_seller_episode,
        EpisodeEffectKindV1::Opened,
        "0.00000000",
        "0",
        "1.00000000",
        "110",
        "0",
        None,
    )?;
    assert_episode_effect(
        entries,
        &expected.reversal_event,
        SELLER,
        0,
        &expected.opening_seller_episode,
        EpisodeEffectKindV1::Closed,
        "2.00000000",
        "220",
        "0.00000000",
        "0",
        "0",
        Some(EpisodeCloseCauseV1::TradeReversal),
    )?;
    assert_episode_effect(
        entries,
        &expected.reversal_event,
        SELLER,
        1,
        &expected.reversal_buyer_episode,
        EpisodeEffectKindV1::Opened,
        "1.00000000",
        "110",
        "0.00000000",
        "0",
        "0",
        None,
    )?;
    assert_episode_effect(
        entries,
        &expected.first_funding_event,
        BUYER,
        0,
        &expected.reversal_seller_episode,
        EpisodeEffectKindV1::Updated,
        "0.00000000",
        "0",
        "0.00000000",
        "0",
        "1.25",
        None,
    )?;
    assert_episode_effect(
        entries,
        &expected.liquidation_fill_event,
        BUYER,
        0,
        &expected.reversal_seller_episode,
        EpisodeEffectKindV1::Interrupted,
        "0.00000000",
        "0",
        "0.00000000",
        "0",
        "0",
        Some(EpisodeCloseCauseV1::LiquidationFill),
    )?;
    assert_episode_effect(
        entries,
        &expected.liquidation_fill_event,
        BUYER,
        1,
        &expected.liquidation_remainder_episode,
        EpisodeEffectKindV1::Opened,
        "0.00000000",
        "0",
        "0.00000000",
        "0",
        "0",
        None,
    )?;
    assert_episode_effect(
        entries,
        &expected.backstop_event,
        BUYER,
        0,
        &expected.liquidation_remainder_episode,
        EpisodeEffectKindV1::Interrupted,
        "0.00000000",
        "0",
        "0.00000000",
        "0",
        "0",
        Some(EpisodeCloseCauseV1::BackstopInterrupted),
    )?;
    assert_episode_effect(
        entries,
        &expected.backstop_event,
        SELLER,
        0,
        &expected.reversal_buyer_episode,
        EpisodeEffectKindV1::Interrupted,
        "0.00000000",
        "0",
        "0.00000000",
        "0",
        "0",
        Some(EpisodeCloseCauseV1::BackstopInterrupted),
    )?;
    assert_episode_effect(
        entries,
        &expected.recovery_event,
        BUYER,
        0,
        &expected.recovery_buyer_episode,
        EpisodeEffectKindV1::Opened,
        "0.25000000",
        "23.75",
        "0.00000000",
        "0",
        "0",
        None,
    )?;
    assert_episode_effect(
        entries,
        &expected.recovery_event,
        SELLER,
        0,
        &expected.recovery_seller_episode,
        EpisodeEffectKindV1::Opened,
        "0.00000000",
        "0",
        "0.25000000",
        "23.75",
        "0",
        None,
    )?;
    assert_episode_effect(
        entries,
        &expected.recovered_funding_event,
        BUYER,
        0,
        &expected.recovery_buyer_episode,
        EpisodeEffectKindV1::Updated,
        "0.00000000",
        "0",
        "0.00000000",
        "0",
        "0.75",
        None,
    )?;
    assert_current_episode(
        entries,
        BUYER,
        &expected.market,
        &expected.recovery_buyer_episode,
    )?;
    assert_current_episode(
        entries,
        SELLER,
        &expected.market,
        &expected.recovery_seller_episode,
    )?;

    for account in [BUYER, SELLER] {
        let key = PositionUnresolvedCauseFactRecordV1::state_key(
            &account,
            &expected.market,
            &expected.backstop_event,
            &expected.liquidation,
        )
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
        let record = decode_at(
            entries,
            &key,
            PositionUnresolvedCauseFactRecordV1::decode_at,
        )?;
        if record.account_id() != account
            || record.market_id() != &expected.market
            || record.event_id() != &expected.backstop_event
            || record.liquidation_id() != &expected.liquidation
            || record.cause() != PositionUnresolvedCauseV1::BackstopLiquidation
        {
            return Err(FixtureRunError::PositionSemanticMismatch);
        }
    }

    let current_key = LiquidationCurrentRecordV1::state_key(&expected.liquidation)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let current = decode_at(entries, &current_key, LiquidationCurrentRecordV1::decode_at)?;
    if current.account_id() != BUYER
        || current.observed_status() != LiquidationObservedStatusV1::BackstopObserved
        || current.start_margin_value() != UsdAmount::from_str("9")?
        || current.start_maintenance_requirement() != UsdAmount::from_str("10")?
        || current.start_event_id() != &expected.liquidation_start_event
        || current.first_backstop_event_id() != Some(&expected.backstop_event)
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    let start_key = LiquidationStartFactRecordV1::state_key(
        &expected.liquidation,
        &expected.liquidation_start_event,
    )
    .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let start = decode_at(entries, &start_key, LiquidationStartFactRecordV1::decode_at)?;
    if start.account_id() != BUYER
        || start.margin_value() != UsdAmount::from_str("9")?
        || start.maintenance_requirement() != UsdAmount::from_str("10")?
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    let fill_key = LiquidationFillFactRecordV1::state_key(
        &expected.liquidation,
        &expected.liquidation_fill_event,
    )
    .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let fill = decode_at(entries, &fill_key, LiquidationFillFactRecordV1::decode_at)?;
    if fill.account_id() != BUYER
        || fill.market_id() != &expected.market
        || fill.price() != Price::parse_at_scale("90", 6)?
        || fill.quantity() != Quantity::parse_at_scale("0.25", 8)?
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    let flow_key = LiquidationMarketFlowCurrentRecordV1::state_key(
        &expected.liquidation,
        &BUYER,
        &expected.market,
    )
    .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let flow = decode_at(
        entries,
        &flow_key,
        LiquidationMarketFlowCurrentRecordV1::decode_at,
    )?;
    if flow.observed_filled_quantity() != Quantity::parse_at_scale("0.25", 8)?
        || flow.first_fill_event_id() != &expected.liquidation_fill_event
        || flow.last_fill_event_id() != &expected.liquidation_fill_event
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    let backstop_key =
        BackstopLiquidationFactRecordV1::state_key(&expected.liquidation, &expected.backstop_event)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let backstop = decode_at(
        entries,
        &backstop_key,
        BackstopLiquidationFactRecordV1::decode_at,
    )?;
    if backstop.account_id() != BUYER
        || backstop.backstop_account_id() != SELLER
        || backstop.market_id() != &expected.market
        || backstop.quantity() != Quantity::parse_at_scale("0.5", 8)?
        || backstop.transfer_price_resolution()
            != LiquidationSourceValueResolutionV1::UnavailableFromSource
        || backstop.entry_price_resolution()
            != LiquidationSourceValueResolutionV1::UnavailableFromSource
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    let settlement_key = PositionSettlementFactRecordV1::state_key(
        &expected.settlement_event,
        &BUYER,
        &expected.market,
    )
    .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let settlement = decode_at(
        entries,
        &settlement_key,
        PositionSettlementFactRecordV1::decode_at,
    )?;
    if settlement.settlement_price() != Price::parse_at_scale("0", 6)?
        || settlement.settled_quantity() != Quantity::parse_at_scale("0.25", 8)?
        || settlement.realized_pnl() != QuoteAmount::from_str("-2.5")?
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    let mut settlement_claim_seen = false;
    for (key, bytes) in entries.iter().filter(|(key, _)| {
        key.namespace().starts_with("position-") || key.namespace().starts_with("account-")
    }) {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
        let Some(raw_pnl) = value.get("realized_pnl") else {
            continue;
        };
        let claimed_pnl = raw_pnl
            .as_str()
            .ok_or(FixtureRunError::PositionSemanticMismatch)
            .and_then(|raw| QuoteAmount::from_str(raw).map_err(FixtureRunError::from))?;
        if claimed_pnl == QuoteAmount::from_str("-2.5")? {
            if key != &settlement_key || settlement_claim_seen {
                return Err(FixtureRunError::PositionSemanticMismatch);
            }
            settlement_claim_seen = true;
        }
    }
    if !settlement_claim_seen {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    if entries.iter().any(|(key, bytes)| {
        (key.namespace() == "position-episode.v1"
            || key.namespace() == "position-episode-effect-fact.v1")
            && bytes
                .windows(expected.interrupted_funding_event.as_str().len())
                .any(|window| window == expected.interrupted_funding_event.as_str().as_bytes())
    }) {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    if namespace_counts(entries) != expected_namespace_counts() {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

fn assert_interrupted_episode(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    episode_id: &PositionEpisodeId,
    account: Address,
    close_event: &EventId,
    close_cause: EpisodeCloseCauseV1,
) -> Result<(), FixtureRunError> {
    let key = PositionEpisodeRecordV1::state_key(episode_id)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let record = decode_at(entries, &key, PositionEpisodeRecordV1::decode_at)?;
    if record.account_id() != account
        || record.status() != EpisodeStatusV1::Interrupted
        || record.close_event_id() != Some(close_event)
        || record.close_cause() != Some(close_cause)
        || record.last_event_id() != close_event
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn assert_episode_snapshot(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    episode_id: &PositionEpisodeId,
    account: Address,
    opening_event: &EventId,
    opening_ordinal: u8,
    opening_position: &str,
    completeness: EpisodeCompletenessV1,
    buy_quantity: &str,
    buy_notional: &str,
    sell_quantity: &str,
    sell_notional: &str,
    funding_paid: &str,
    status: EpisodeStatusV1,
    close_event: &EventId,
    close_cause: EpisodeCloseCauseV1,
) -> Result<(), FixtureRunError> {
    let key = PositionEpisodeRecordV1::state_key(episode_id)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let record = decode_at(entries, &key, PositionEpisodeRecordV1::decode_at)?;
    if record.episode_id() != episode_id
        || record.account_id() != account
        || record.market_id().as_str() != MARKET
        || record.opening_anchor_event_id() != opening_event
        || record.opening_leg_ordinal() != opening_ordinal
        || record.opening_position() != PositionQuantity::from_str(opening_position)?
        || record.completeness() != completeness
        || record.buy_quantity() != Quantity::from_str(buy_quantity)?
        || record.buy_notional().to_string() != buy_notional
        || record.sell_quantity() != Quantity::from_str(sell_quantity)?
        || record.sell_notional().to_string() != sell_notional
        || record.funding_paid() != QuoteAmount::from_str(funding_paid)?
        || record.status() != status
        || record.close_event_id() != Some(close_event)
        || record.close_cause() != Some(close_cause)
        || record.last_event_id() != close_event
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn assert_episode_effect(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    event_id: &EventId,
    account: Address,
    ordinal: u8,
    episode_id: &PositionEpisodeId,
    effect_kind: EpisodeEffectKindV1,
    buy_quantity: &str,
    buy_notional: &str,
    sell_quantity: &str,
    sell_notional: &str,
    funding_paid: &str,
    close_cause: Option<EpisodeCloseCauseV1>,
) -> Result<(), FixtureRunError> {
    let market = MarketId::new(MARKET)?;
    let key = PositionEpisodeEffectFactRecordV1::state_key(event_id, &account, &market, ordinal)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let record = decode_at(entries, &key, PositionEpisodeEffectFactRecordV1::decode_at)?;
    if record.event_id() != event_id
        || record.account_id() != account
        || record.market_id() != &market
        || record.leg_ordinal() != ordinal
        || record.episode_id() != episode_id
        || record.effect_kind() != effect_kind
        || record.buy_quantity_delta() != Quantity::from_str(buy_quantity)?
        || record.buy_notional_delta().to_string() != buy_notional
        || record.sell_quantity_delta() != Quantity::from_str(sell_quantity)?
        || record.sell_notional_delta().to_string() != sell_notional
        || record.funding_paid_delta() != QuoteAmount::from_str(funding_paid)?
        || record.close_cause() != close_cause
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

fn assert_quantity(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    account: Address,
    market: &MarketId,
    known: Option<&str>,
    last_event: &EventId,
) -> Result<(), FixtureRunError> {
    let key = PositionQuantityCurrentRecordV1::state_key(&account, market)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let record = decode_at(entries, &key, PositionQuantityCurrentRecordV1::decode_at)?;
    let expected_quantity = known
        .map(PositionQuantity::from_str)
        .transpose()
        .map_err(FixtureRunError::from)?;
    if record.account_id() != account
        || record.market_id() != market
        || record.known_quantity() != expected_quantity
        || record.last_event_id() != last_event
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn assert_position_effect(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    trade_id: &str,
    role: TradeParticipantRoleV1,
    account: Address,
    start: &str,
    result: &str,
    transition: PositionAnchorTransitionV1,
) -> Result<(), FixtureRunError> {
    let trade_id = TradeId::new(trade_id)?;
    let key = PositionEffectFactRecordV1::state_key(&trade_id, role)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let record = decode_at(entries, &key, PositionEffectFactRecordV1::decode_at)?;
    if record.trade_id() != &trade_id
        || record.account_id() != account
        || record.market_id().as_str() != MARKET
        || record.role() != role
        || record.anchor_transition() != transition
        || record.start_position() != PositionQuantity::from_str(start)?
        || record.result_position() != PositionQuantity::from_str(result)?
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn assert_open_episode(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    episode_id: &PositionEpisodeId,
    account: Address,
    opening_event: &EventId,
    completeness: EpisodeCompletenessV1,
    last_event: &EventId,
    buy_quantity: &str,
    buy_notional: &str,
    sell_quantity: &str,
    sell_notional: &str,
    funding_paid: &str,
) -> Result<(), FixtureRunError> {
    let key = PositionEpisodeRecordV1::state_key(episode_id)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let record = decode_at(entries, &key, PositionEpisodeRecordV1::decode_at)?;
    if record.episode_id() != episode_id
        || record.account_id() != account
        || record.market_id().as_str() != MARKET
        || record.opening_anchor_event_id() != opening_event
        || record.completeness() != completeness
        || record.last_event_id() != last_event
        || record.buy_quantity() != Quantity::from_str(buy_quantity)?
        || record.buy_notional().to_string() != buy_notional
        || record.sell_quantity() != Quantity::from_str(sell_quantity)?
        || record.sell_notional().to_string() != sell_notional
        || record.funding_paid() != QuoteAmount::from_str(funding_paid)?
        || record.status() != EpisodeStatusV1::Open
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

fn assert_current_episode(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    account: Address,
    market: &MarketId,
    episode_id: &PositionEpisodeId,
) -> Result<(), FixtureRunError> {
    let key = PositionEpisodeCurrentRecordV1::state_key(&account, market)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let record = decode_at(entries, &key, PositionEpisodeCurrentRecordV1::decode_at)?;
    if record.account_id() != account
        || record.market_id() != market
        || record.episode_id() != Some(episode_id)
        || record.attribution_resolution() != EpisodeAttributionResolutionV1::Resolved
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

fn decode_at<T>(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    key: &StateKey,
    decode: impl FnOnce(&StateKey, &[u8]) -> Result<T, canonical_ledger::PositionStateError>,
) -> Result<T, FixtureRunError> {
    entries
        .get(key)
        .ok_or(FixtureRunError::PositionSemanticMismatch)
        .and_then(|bytes| decode(key, bytes).map_err(|_| FixtureRunError::PositionSemanticMismatch))
}

fn namespace_count(entries: &BTreeMap<StateKey, Vec<u8>>, namespace: &str) -> usize {
    entries
        .keys()
        .filter(|key| key.namespace() == namespace)
        .count()
}

pub(super) fn namespace_counts(entries: &BTreeMap<StateKey, Vec<u8>>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for key in entries.keys() {
        *counts.entry(key.namespace().to_owned()).or_insert(0) += 1;
    }
    counts
}

fn expected_namespace_counts() -> BTreeMap<String, usize> {
    BTreeMap::from([
        ("account-fact.v1".to_owned(), 3),
        ("account-quote-flow-current.v1".to_owned(), 1),
        ("asset-context-current.v1".to_owned(), 2),
        ("backstop-liquidation-fact.v1".to_owned(), 1),
        ("dex-current.v1".to_owned(), 1),
        ("liquidation-current.v1".to_owned(), 1),
        ("liquidation-fill-fact.v1".to_owned(), 1),
        ("liquidation-market-flow-current.v1".to_owned(), 1),
        ("liquidation-start-fact.v1".to_owned(), 1),
        ("market-current.v1".to_owned(), 1),
        ("market-fact.v1".to_owned(), 4),
        ("market-metadata-version.v1".to_owned(), 1),
        ("order-current.v1".to_owned(), 6),
        ("order-fact.v1".to_owned(), 6),
        ("order-transition.v1".to_owned(), 6),
        ("position-effect-fact.v1".to_owned(), 6),
        ("position-episode-current.v1".to_owned(), 2),
        ("position-episode-effect-fact.v1".to_owned(), 14),
        ("position-episode.v1".to_owned(), 7),
        ("position-quantity-current.v1".to_owned(), 2),
        ("position-settlement-fact.v1".to_owned(), 1),
        ("position-unresolved-cause-fact.v1".to_owned(), 2),
        ("reconciliation.v1".to_owned(), 3),
        ("trade-participant.v1".to_owned(), 6),
        ("trade-participant.v2".to_owned(), 6),
        ("trade-reconciliation.v2".to_owned(), 3),
        ("trade.v1".to_owned(), 3),
        ("trade.v2".to_owned(), 3),
    ])
}

pub(super) fn fixture_oracle(
    expected: &FrozenExpectation,
) -> Result<serde_json::Value, FixtureRunError> {
    let recovery_trade = TradeId::new(RECOVERY_TRADE)?;
    let buyer_quantity = PositionQuantityCurrentRecordV1::state_key(&BUYER, &expected.market)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let seller_quantity = PositionQuantityCurrentRecordV1::state_key(&SELLER, &expected.market)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let recovery_buyer_effect =
        PositionEffectFactRecordV1::state_key(&recovery_trade, TradeParticipantRoleV1::Buyer)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let recovery_seller_effect =
        PositionEffectFactRecordV1::state_key(&recovery_trade, TradeParticipantRoleV1::Seller)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let buyer_cause = PositionUnresolvedCauseFactRecordV1::state_key(
        &BUYER,
        &expected.market,
        &expected.backstop_event,
        &expected.liquidation,
    )
    .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let seller_cause = PositionUnresolvedCauseFactRecordV1::state_key(
        &SELLER,
        &expected.market,
        &expected.backstop_event,
        &expected.liquidation,
    )
    .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let settlement = PositionSettlementFactRecordV1::state_key(
        &expected.settlement_event,
        &BUYER,
        &expected.market,
    )
    .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let key = |value: &StateKey| {
        serde_json::json!({
            "namespace": value.namespace(),
            "key_hex": hex::encode(value.key()),
        })
    };
    Ok(serde_json::json!({
        "order_ids": {
            "opening_buyer": "position-open-buyer-order",
            "opening_seller": "position-open-seller-order",
            "reversal_buyer": "position-reversal-buyer-order",
            "reversal_seller": "position-reversal-seller-order",
            "recovery_buyer": "position-recovery-buyer-order",
            "recovery_seller": "position-recovery-seller-order",
        },
        "transaction_ids": {
            "opening_trade": "state-replay-position-open",
            "reversal_trade": "state-replay-position-reversal",
            "first_funding": "state-replay-position-first-funding",
            "liquidation_start": "state-replay-position-liquidation-start",
            "liquidation_fill": "state-replay-position-liquidation-fill",
            "backstop": "state-replay-position-backstop",
            "interrupted_funding": "state-replay-position-interrupted-funding",
            "settlement": "state-replay-position-settlement",
            "recovery_trade": "state-replay-position-recovery",
            "recovered_funding": "state-replay-position-recovered-funding",
        },
        "event_ids": {
            "opening_trade": expected.opening_event.as_str(),
            "reversal_trade": expected.reversal_event.as_str(),
            "first_funding": expected.first_funding_event.as_str(),
            "liquidation_start": expected.liquidation_start_event.as_str(),
            "liquidation_fill": expected.liquidation_fill_event.as_str(),
            "backstop": expected.backstop_event.as_str(),
            "interrupted_funding": expected.interrupted_funding_event.as_str(),
            "settlement": expected.settlement_event.as_str(),
            "recovery_trade": expected.recovery_event.as_str(),
            "recovered_funding": expected.recovered_funding_event.as_str(),
        },
        "episode_ids": {
            "opening_buyer": expected.opening_buyer_episode.as_str(),
            "opening_seller": expected.opening_seller_episode.as_str(),
            "reversal_buyer": expected.reversal_buyer_episode.as_str(),
            "reversal_seller": expected.reversal_seller_episode.as_str(),
            "liquidation_remainder": expected.liquidation_remainder_episode.as_str(),
            "recovery_buyer": expected.recovery_buyer_episode.as_str(),
            "recovery_seller": expected.recovery_seller_episode.as_str(),
        },
        "state_keys": {
            "buyer_quantity_current": key(&buyer_quantity),
            "seller_quantity_current": key(&seller_quantity),
            "recovery_buyer_effect": key(&recovery_buyer_effect),
            "recovery_seller_effect": key(&recovery_seller_effect),
            "buyer_unresolved_cause": key(&buyer_cause),
            "seller_unresolved_cause": key(&seller_cause),
            "settlement_fact": key(&settlement),
        },
        "notionals": {
            "opening_buyer_buy": "200",
            "opening_seller_sell": "200",
            "reversal_buyer_close_buy": "220",
            "reversal_buyer_open_buy": "110",
            "reversal_seller_close_sell": "220",
            "reversal_seller_open_sell": "110",
            "recovery_buyer_buy": "23.75",
            "recovery_seller_sell": "23.75",
        },
    }))
}
