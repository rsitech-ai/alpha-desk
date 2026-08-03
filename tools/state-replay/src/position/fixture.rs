use super::*;

#[derive(Debug)]
pub(super) struct Scenario {
    pub(super) blocks: Vec<BlockEnvelope>,
    pub(super) expected: FrozenExpectation,
    pub(super) checkpoint_height: u64,
    pub(super) end_height: u64,
}

#[derive(Debug)]
pub(super) struct FrozenExpectation {
    pub(super) market: MarketId,
    pub(super) liquidation: LiquidationId,
    pub(super) opening_event: EventId,
    pub(super) reversal_event: EventId,
    pub(super) first_funding_event: EventId,
    pub(super) liquidation_start_event: EventId,
    pub(super) liquidation_fill_event: EventId,
    pub(super) backstop_event: EventId,
    pub(super) interrupted_funding_event: EventId,
    pub(super) settlement_event: EventId,
    pub(super) recovery_event: EventId,
    pub(super) recovered_funding_event: EventId,
    pub(super) opening_buyer_episode: PositionEpisodeId,
    pub(super) opening_seller_episode: PositionEpisodeId,
    pub(super) reversal_buyer_episode: PositionEpisodeId,
    pub(super) reversal_seller_episode: PositionEpisodeId,
    pub(super) liquidation_remainder_episode: PositionEpisodeId,
    pub(super) recovery_buyer_episode: PositionEpisodeId,
    pub(super) recovery_seller_episode: PositionEpisodeId,
}

pub(super) fn build_scenario(
    config: &PositionRunConfig,
    chain: &ChainId,
    settlement_pnl: &str,
) -> Result<Scenario, FixtureRunError> {
    let checkpoint_height = START_HEIGHT + config.checkpoint_after - 1;
    let end_height = START_HEIGHT + config.blocks - 1;
    let market = MarketId::new(MARKET)?;
    let liquidation = LiquidationId::new(LIQUIDATION)?;
    let mut blocks = Vec::with_capacity(
        usize::try_from(config.blocks).map_err(|_| FixtureRunError::InvalidConfig)?,
    );
    let mut opening_event = None;
    let mut reversal_event = None;
    let mut first_funding_event = None;
    let mut liquidation_start_event = None;
    let mut liquidation_fill_event = None;
    let mut backstop_event = None;
    let mut interrupted_funding_event = None;
    let mut settlement_event = None;
    let mut recovery_event = None;
    let mut recovered_funding_event = None;

    for height in START_HEIGHT..=end_height {
        let mut events = if height == START_HEIGHT {
            account::market_prerequisite_events(height)?
        } else {
            Vec::new()
        };
        if height == checkpoint_height {
            events.extend(order_prerequisites(height, events.len() as u32, &market)?);
            let event = trade_event(
                height,
                events.len() as u32,
                OPEN_TRADE,
                "position-open",
                BUYER,
                SELLER,
                "0.00000000",
                "0.00000000",
                "position-open-buyer-order",
                "position-open-seller-order",
                "100.000000",
                "2.00000000",
                "1.0.0",
            )?;
            opening_event = Some(event.event_id().clone());
            events.push(event);
        } else if height == checkpoint_height + 1 {
            let event = trade_event(
                height,
                0,
                REVERSAL_TRADE,
                "position-reversal",
                SELLER,
                BUYER,
                "-2.00000000",
                "2.00000000",
                "position-reversal-buyer-order",
                "position-reversal-seller-order",
                "110.000000",
                "3.00000000",
                "1.0.0",
            )?;
            reversal_event = Some(event.event_id().clone());
            events.push(event);
        } else if height == checkpoint_height + 2 {
            let event = position_event(
                height,
                0,
                "position-first-funding",
                EventPayload::FundingPaid(FundingPaid {
                    account_id: BUYER,
                    market_id: market.clone(),
                    amount: QuoteAmount::from_str("1.25")?,
                    funding_rate: FundingRate::from_str("0.0001")?,
                }),
                vec![market.clone()],
                vec![BUYER],
                "1.0.0",
            )?;
            first_funding_event = Some(event.event_id().clone());
            events.push(event);
        } else if height == checkpoint_height + 3 {
            let event = position_event(
                height,
                0,
                "position-liquidation-start",
                EventPayload::LiquidationStarted(LiquidationStarted {
                    account_id: BUYER,
                    liquidation_id: liquidation.clone(),
                    margin_value: UsdAmount::from_str("9")?,
                    maintenance_requirement: UsdAmount::from_str("10")?,
                }),
                vec![],
                vec![BUYER],
                "1.0.0",
            )?;
            liquidation_start_event = Some(event.event_id().clone());
            events.push(event);
        } else if height == checkpoint_height + 4 {
            let event = position_event(
                height,
                0,
                "position-liquidation-fill",
                EventPayload::LiquidationFill(LiquidationFill {
                    liquidation_id: liquidation.clone(),
                    account_id: BUYER,
                    market_id: market.clone(),
                    price: Price::parse_at_scale("90", 6)?,
                    quantity: Quantity::parse_at_scale("0.25", 8)?,
                }),
                vec![market.clone()],
                vec![BUYER],
                "1.0.0",
            )?;
            liquidation_fill_event = Some(event.event_id().clone());
            events.push(event);
        } else if height == checkpoint_height + 5 {
            let backstop = position_event(
                height,
                0,
                "position-backstop",
                EventPayload::BackstopLiquidation(BackstopLiquidation {
                    liquidation_id: liquidation.clone(),
                    account_id: BUYER,
                    backstop_account_id: SELLER,
                    market_id: market.clone(),
                    quantity: Quantity::parse_at_scale("0.5", 8)?,
                }),
                vec![market.clone()],
                vec![BUYER, SELLER],
                "1.0.0",
            )?;
            backstop_event = Some(backstop.event_id().clone());
            events.push(backstop);
        } else if height == checkpoint_height + 6 {
            let funding = position_event(
                height,
                0,
                "position-interrupted-funding",
                EventPayload::FundingPaid(FundingPaid {
                    account_id: BUYER,
                    market_id: market.clone(),
                    amount: QuoteAmount::from_str("0.5")?,
                    funding_rate: FundingRate::from_str("0.0001")?,
                }),
                vec![market.clone()],
                vec![BUYER],
                "1.0.0",
            )?;
            interrupted_funding_event = Some(funding.event_id().clone());
            events.push(funding);
        } else if height == checkpoint_height + 7 {
            let event = position_event(
                height,
                0,
                "position-settlement",
                EventPayload::PositionSettled(PositionSettled {
                    account_id: BUYER,
                    market_id: market.clone(),
                    settlement_price: Price::parse_at_scale("0", 6)?,
                    settled_quantity: Quantity::parse_at_scale("0.25", 8)?,
                    realized_pnl: QuoteAmount::from_str(settlement_pnl)?,
                }),
                vec![market.clone()],
                vec![BUYER],
                "1.0.0",
            )?;
            settlement_event = Some(event.event_id().clone());
            events.push(event);
        } else if height == checkpoint_height + 8 {
            let recovery = trade_event(
                height,
                0,
                RECOVERY_TRADE,
                "position-recovery",
                BUYER,
                SELLER,
                "4.00000000",
                "0.00000000",
                "position-recovery-buyer-order",
                "position-recovery-seller-order",
                "95.000000",
                "0.25000000",
                "1.0.0",
            )?;
            recovery_event = Some(recovery.event_id().clone());
            events.push(recovery);
            let funding = position_event(
                height,
                1,
                "position-recovered-funding",
                EventPayload::FundingPaid(FundingPaid {
                    account_id: BUYER,
                    market_id: market.clone(),
                    amount: QuoteAmount::from_str("0.75")?,
                    funding_rate: FundingRate::from_str("0.0001")?,
                }),
                vec![market.clone()],
                vec![BUYER],
                "1.0.0",
            )?;
            recovered_funding_event = Some(funding.event_id().clone());
            events.push(funding);
        }
        blocks.push(account::block(height, chain, events)?);
    }

    let opening_event = opening_event.ok_or(FixtureRunError::InvalidConfig)?;
    let reversal_event = reversal_event.ok_or(FixtureRunError::InvalidConfig)?;
    let recovery_event = recovery_event.ok_or(FixtureRunError::InvalidConfig)?;
    let expected = FrozenExpectation {
        market: market.clone(),
        liquidation,
        opening_buyer_episode: derive_position_episode_id(&BUYER, &market, &opening_event, 0)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?,
        opening_seller_episode: derive_position_episode_id(&SELLER, &market, &opening_event, 0)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?,
        reversal_buyer_episode: derive_position_episode_id(&SELLER, &market, &reversal_event, 1)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?,
        reversal_seller_episode: derive_position_episode_id(&BUYER, &market, &reversal_event, 1)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?,
        liquidation_remainder_episode: derive_position_episode_id(
            &BUYER,
            &market,
            liquidation_fill_event
                .as_ref()
                .ok_or(FixtureRunError::InvalidConfig)?,
            1,
        )
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?,
        recovery_buyer_episode: derive_position_episode_id(&BUYER, &market, &recovery_event, 0)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?,
        recovery_seller_episode: derive_position_episode_id(&SELLER, &market, &recovery_event, 0)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?,
        opening_event,
        reversal_event,
        first_funding_event: first_funding_event.ok_or(FixtureRunError::InvalidConfig)?,
        liquidation_start_event: liquidation_start_event.ok_or(FixtureRunError::InvalidConfig)?,
        liquidation_fill_event: liquidation_fill_event.ok_or(FixtureRunError::InvalidConfig)?,
        backstop_event: backstop_event.ok_or(FixtureRunError::InvalidConfig)?,
        interrupted_funding_event: interrupted_funding_event
            .ok_or(FixtureRunError::InvalidConfig)?,
        settlement_event: settlement_event.ok_or(FixtureRunError::InvalidConfig)?,
        recovery_event,
        recovered_funding_event: recovered_funding_event.ok_or(FixtureRunError::InvalidConfig)?,
    };
    Ok(Scenario {
        blocks,
        expected,
        checkpoint_height,
        end_height,
    })
}

fn order_prerequisites(
    height: u64,
    start_index: u32,
    market: &MarketId,
) -> Result<Vec<CanonicalEventEnvelope>, FixtureRunError> {
    let specs = [
        ("position-open-buyer-order", BUYER, OrderSide::Buy, "2"),
        ("position-open-seller-order", SELLER, OrderSide::Sell, "2"),
        ("position-reversal-buyer-order", SELLER, OrderSide::Buy, "3"),
        (
            "position-reversal-seller-order",
            BUYER,
            OrderSide::Sell,
            "3",
        ),
        (
            "position-recovery-buyer-order",
            BUYER,
            OrderSide::Buy,
            "0.25",
        ),
        (
            "position-recovery-seller-order",
            SELLER,
            OrderSide::Sell,
            "0.25",
        ),
    ];
    specs
        .into_iter()
        .enumerate()
        .map(|(offset, (order_id, account_id, side, quantity))| {
            position_event(
                height,
                start_index + u32::try_from(offset).map_err(|_| FixtureRunError::InvalidConfig)?,
                order_id,
                EventPayload::OrderAccepted(OrderAccepted {
                    order_id: OrderId::new(order_id)?,
                    account_id,
                    market_id: market.clone(),
                    side,
                    limit_price: Price::parse_at_scale("100", 6)?,
                    quantity: Quantity::parse_at_scale(quantity, 8)?,
                }),
                vec![market.clone()],
                vec![account_id],
                "1.0.0",
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn trade_event(
    height: u64,
    index: u32,
    trade_id: &str,
    transaction: &str,
    buyer: Address,
    seller: Address,
    buyer_start: &str,
    seller_start: &str,
    buyer_order: &str,
    seller_order: &str,
    price: &str,
    quantity: &str,
    schema: &str,
) -> Result<CanonicalEventEnvelope, FixtureRunError> {
    let market = MarketId::new(MARKET)?;
    position_event(
        height,
        index,
        transaction,
        EventPayload::TradeMatched(TradeMatched {
            trade_id: Some(TradeId::new(trade_id)?),
            market_id: Some(market.clone()),
            maker_order_id: Some(OrderId::new(seller_order)?),
            taker_order_id: Some(OrderId::new(buyer_order)?),
            price: Price::parse_at_scale(price, 6)?,
            quantity: Quantity::parse_at_scale(quantity, 8)?,
            deterministic_seed: height,
            participants: Some(Box::new([
                TradeParticipantV1 {
                    role: TradeParticipantRoleV1::Buyer,
                    account_id: buyer,
                    start_position: PositionQuantity::from_str(buyer_start)?,
                    order_id: OrderId::new(buyer_order)?,
                    twap_id: None,
                    client_order_id: None,
                },
                TradeParticipantV1 {
                    role: TradeParticipantRoleV1::Seller,
                    account_id: seller,
                    start_position: PositionQuantity::from_str(seller_start)?,
                    order_id: OrderId::new(seller_order)?,
                    twap_id: None,
                    client_order_id: None,
                },
            ])),
        }),
        vec![market],
        vec![buyer, seller],
        schema,
    )
}

fn position_event(
    height: u64,
    index: u32,
    transaction: &str,
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
        transaction_id: TransactionId::new(format!("state-replay-{transaction}"))?,
        transaction_index: index,
        canonical_event_index: 0,
        market_ids,
        account_ids,
        source_evidence: vec![SourceEvidence::try_new_indexed(
            SourceId::new("state-replay-position")?,
            "v1",
            height.to_string(),
            payload_hash,
            index,
        )?],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(time.unix_micros())?,
        ingested_at: KnownTime::from_unix_micros(time.unix_micros())?,
        canonicalized_at: KnownTime::from_unix_micros(time.unix_micros())?,
        parser_version: "state-replay-position-fixture-v1".to_owned(),
        payload,
    })?)
}
