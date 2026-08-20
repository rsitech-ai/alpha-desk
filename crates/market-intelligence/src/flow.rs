use domain_types::{BasisPoints, EntityId, ProbabilityPpm, UsdAmount};
use serde::{Deserialize, Serialize};

use crate::{
    MarketError,
    hash::digest,
    math::{product_ppm, require_matching_usd_scale, robust_z_milli, scale_usd_by_ppm},
    normalization::LiquidityNormalizer,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RiskFlowKind {
    OpenLong,
    AddLong,
    ReduceLong,
    CloseLong,
    OpenShort,
    AddShort,
    ReduceShort,
    CloseShort,
    Static,
}

impl RiskFlowKind {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::OpenLong => "open_long",
            Self::AddLong => "add_long",
            Self::ReduceLong => "reduce_long",
            Self::CloseLong => "close_long",
            Self::OpenShort => "open_short",
            Self::AddShort => "add_short",
            Self::ReduceShort => "reduce_short",
            Self::CloseShort => "close_short",
            Self::Static => "static",
        }
    }

    #[must_use]
    pub const fn signed_new_risk_sign(self) -> i8 {
        match self {
            Self::OpenLong | Self::AddLong => 1,
            Self::ReduceLong | Self::CloseLong => -1,
            Self::OpenShort | Self::AddShort => -1,
            Self::ReduceShort | Self::CloseShort => 1,
            Self::Static => 0,
        }
    }

    #[must_use]
    pub const fn is_close_risk(self) -> bool {
        matches!(self, Self::CloseLong | Self::CloseShort)
    }

    #[must_use]
    pub const fn is_opening_or_add(self) -> bool {
        matches!(
            self,
            Self::OpenLong | Self::AddLong | Self::OpenShort | Self::AddShort
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartFlowContribution {
    pub subject: EntityId,
    pub kind: RiskFlowKind,
    pub notional_usd: UsdAmount,
    pub skill_probability: ProbabilityPpm,
    pub expected_edge_after_cost_bps: BasisPoints,
    pub regime_fit: ProbabilityPpm,
    pub copyability: ProbabilityPpm,
    pub independence_weight: ProbabilityPpm,
    pub data_confidence: ProbabilityPpm,
    pub freshness_decay: ProbabilityPpm,
    pub intent_adjustment: ProbabilityPpm,
}

impl SmartFlowContribution {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        subject: EntityId,
        kind: RiskFlowKind,
        notional_usd: UsdAmount,
        skill_probability: ProbabilityPpm,
        expected_edge_after_cost_bps: BasisPoints,
        regime_fit: ProbabilityPpm,
        copyability: ProbabilityPpm,
        independence_weight: ProbabilityPpm,
        data_confidence: ProbabilityPpm,
        freshness_decay: ProbabilityPpm,
        intent_adjustment: ProbabilityPpm,
    ) -> Result<Self, MarketError> {
        if notional_usd.raw() < 0 {
            return Err(MarketError::Malformed {
                what: "smart_flow",
                reason: "notional must be non-negative",
            });
        }
        Ok(Self {
            subject,
            kind,
            notional_usd,
            skill_probability,
            expected_edge_after_cost_bps,
            regime_fit,
            copyability,
            independence_weight,
            data_confidence,
            freshness_decay,
            intent_adjustment,
        })
    }

    pub fn signed_new_risk_usd(&self) -> Result<UsdAmount, MarketError> {
        if matches!(
            self.kind,
            RiskFlowKind::Static | RiskFlowKind::CloseLong | RiskFlowKind::CloseShort
        ) {
            return UsdAmount::from_raw(0, self.notional_usd.scale()).map_err(Into::into);
        }
        let sign = i128::from(self.kind.signed_new_risk_sign());
        UsdAmount::from_raw(
            self.notional_usd
                .raw()
                .checked_mul(sign)
                .ok_or(MarketError::Overflow)?,
            self.notional_usd.scale(),
        )
        .map_err(Into::into)
    }

    pub fn close_risk_usd(&self) -> Result<UsdAmount, MarketError> {
        if self.kind.is_close_risk() {
            Ok(self.notional_usd)
        } else {
            UsdAmount::from_raw(0, self.notional_usd.scale()).map_err(Into::into)
        }
    }

    pub fn edge_weight(&self) -> Result<ProbabilityPpm, MarketError> {
        let bps = self.expected_edge_after_cost_bps.raw();
        let ppm = 500_000_i128
            .checked_add(bps.checked_mul(5_000).ok_or(MarketError::Overflow)?)
            .ok_or(MarketError::Overflow)?
            .clamp(0, 1_000_000);
        ProbabilityPpm::from_ppm(u32::try_from(ppm).map_err(|_| MarketError::Overflow)?)
            .map_err(Into::into)
    }

    pub fn weighted_usd(&self) -> Result<UsdAmount, MarketError> {
        let signed = self.signed_new_risk_usd()?;
        let weight = product_ppm(&[
            self.skill_probability,
            self.edge_weight()?,
            self.regime_fit,
            self.copyability,
            self.independence_weight,
            self.data_confidence,
            self.freshness_decay,
            self.intent_adjustment,
        ])?;
        scale_usd_by_ppm(signed, weight)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeightedContributionEvidence {
    pub subject: EntityId,
    pub kind: RiskFlowKind,
    pub signed_new_risk_usd: UsdAmount,
    pub close_risk_usd: UsdAmount,
    pub weight_ppm: ProbabilityPpm,
    pub weighted_usd: UsdAmount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartFlowAggregate {
    pub raw_usd: UsdAmount,
    pub liquidity_normalized_usd: UsdAmount,
    pub robust_z_milli: i64,
    pub independent_votes_milli: u64,
    pub contributions: Vec<WeightedContributionEvidence>,
    pub provenance_hash: [u8; 32],
}

pub fn accumulate_smart_flow(
    contributions: &[SmartFlowContribution],
    normalizer: &LiquidityNormalizer,
    historical_raw_usd: &[i128],
) -> Result<SmartFlowAggregate, MarketError> {
    if contributions.is_empty() {
        return Err(MarketError::InsufficientHistory { what: "smart_flow" });
    }
    let scale = contributions[0].notional_usd.scale();
    let mut raw = UsdAmount::from_raw(0, scale)?;
    let mut votes = 0_u64;
    let mut evidence = Vec::with_capacity(contributions.len());
    for contribution in contributions {
        require_matching_usd_scale(contribution.notional_usd, raw)?;
        let signed = contribution.signed_new_risk_usd()?;
        let close = contribution.close_risk_usd()?;
        let weighted = contribution.weighted_usd()?;
        let weight = product_ppm(&[
            contribution.skill_probability,
            contribution.edge_weight()?,
            contribution.regime_fit,
            contribution.copyability,
            contribution.independence_weight,
            contribution.data_confidence,
            contribution.freshness_decay,
            contribution.intent_adjustment,
        ])?;
        raw = raw.checked_add(weighted)?;
        votes = votes
            .checked_add(u64::from(contribution.independence_weight.ppm()))
            .ok_or(MarketError::Overflow)?;
        evidence.push(WeightedContributionEvidence {
            subject: contribution.subject.clone(),
            kind: contribution.kind,
            signed_new_risk_usd: signed,
            close_risk_usd: close,
            weight_ppm: weight,
            weighted_usd: weighted,
        });
    }
    let normalized = normalizer.normalize(raw)?;
    let z = if historical_raw_usd.is_empty() {
        0
    } else {
        robust_z_milli(raw.raw(), historical_raw_usd)?
    };
    let provenance_hash = digest(&[
        &raw.raw().to_le_bytes(),
        &normalized.raw().to_le_bytes(),
        &z.to_le_bytes(),
        &votes.to_le_bytes(),
        normalizer.version.as_bytes(),
    ]);
    Ok(SmartFlowAggregate {
        raw_usd: raw,
        liquidity_normalized_usd: normalized,
        robust_z_milli: z,
        independent_votes_milli: votes,
        contributions: evidence,
        provenance_hash,
    })
}
