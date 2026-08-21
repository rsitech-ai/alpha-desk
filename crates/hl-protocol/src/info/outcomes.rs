use domain_types::OutcomeId;
use serde_json::Value;

use super::decode::{
    InfoObservationKind, child, expect_capability, malformed, parse_family, require_array_field,
    require_object, require_str, require_u64,
};
use super::{InfoError, InfoParseContext, ParsedInfoResponse};

pub const OUTCOME_SIDE_NAMES: &[&str] = &["Yes", "No"];

pub const OUTCOME_META_KNOWN_FIELDS: &[&str] = &[
    "/outcomes",
    "/outcomes/outcome",
    "/outcomes/name",
    "/outcomes/description",
    "/outcomes/sideSpecs",
    "/outcomes/sideSpecs/name",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeSide {
    name: String,
}

impl OutcomeSide {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeMarket {
    id: OutcomeId,
    raw_id: u64,
    name: String,
    description: String,
    sides: Vec<OutcomeSide>,
}

impl OutcomeMarket {
    #[must_use]
    pub const fn id(&self) -> &OutcomeId {
        &self.id
    }

    #[must_use]
    pub const fn raw_id(&self) -> u64 {
        self.raw_id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    #[must_use]
    pub fn sides(&self) -> &[OutcomeSide] {
        &self.sides
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeMeta {
    outcomes: Vec<OutcomeMarket>,
}

impl OutcomeMeta {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn outcomes(&self) -> &[OutcomeMarket] {
        &self.outcomes
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for OutcomeMeta {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.outcome_meta"])?;
        let object = require_object(parsed.value(), "")?;
        let outcomes = require_array_field(object, "", "outcomes")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/outcomes/{index}");
                let object = require_object(value, &path)?;
                let raw_id = require_u64(object, &path, "outcome")?;
                let sides_path = child(&path, "sideSpecs");
                let sides = require_array_field(object, &path, "sideSpecs")?
                    .iter()
                    .enumerate()
                    .map(|(side_index, side)| {
                        let side_path = format!("{sides_path}/{side_index}");
                        let side_object = require_object(side, &side_path)?;
                        let name = require_str(side_object, &side_path, "name")?.to_owned();
                        if !OUTCOME_SIDE_NAMES.contains(&name.as_str()) {
                            return Err(InfoError::UnknownStateAffectingVariant {
                                path: child(&side_path, "name"),
                                value: name,
                            });
                        }
                        Ok(OutcomeSide { name })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(OutcomeMarket {
                    id: OutcomeId::new(raw_id.to_string())
                        .map_err(|_| malformed(&child(&path, "outcome"), "invalid outcome id"))?,
                    raw_id,
                    name: require_str(object, &path, "name")?.to_owned(),
                    description: require_str(object, &path, "description")?.to_owned(),
                    sides,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { outcomes })
    }
}

pub fn parse_outcome_meta(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, OutcomeMeta), InfoError> {
    parse_family(
        "official.info.outcome_meta",
        raw,
        context,
        OUTCOME_META_KNOWN_FIELDS,
        &[],
    )
}
