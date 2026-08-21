use domain_types::Address;
use serde_json::Value;

use super::decode::{
    InfoObservationKind, address_from_str, expect_capability, malformed, optional_u64,
    parse_family, require_address, require_array, require_object, require_str,
};
use super::{InfoError, InfoParseContext, ParsedInfoResponse};

pub const EXTRA_AGENT_KNOWN_FIELDS: &[&str] = &["/address", "/name", "/validUntil"];
pub const APPROVED_BUILDER_KNOWN_FIELDS: &[&str] = &[];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraAgent {
    address: Address,
    name: String,
    valid_until_millis: Option<u64>,
}

impl ExtraAgent {
    #[must_use]
    pub const fn address(&self) -> Address {
        self.address
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn valid_until_millis(&self) -> Option<u64> {
        self.valid_until_millis
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraAgents {
    agents: Vec<ExtraAgent>,
}

impl ExtraAgents {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn agents(&self) -> &[ExtraAgent] {
        &self.agents
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for ExtraAgents {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.extra_agents"])?;
        let agents = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/{index}");
                let object = require_object(value, &path)?;
                Ok(ExtraAgent {
                    address: require_address(object, &path, "address")?,
                    name: require_str(object, &path, "name")?.to_owned(),
                    valid_until_millis: optional_u64(object, &path, "validUntil")?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { agents })
    }
}

pub fn parse_extra_agents(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, ExtraAgents), InfoError> {
    parse_family(
        "official.info.extra_agents",
        raw,
        context,
        EXTRA_AGENT_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedBuilders {
    builders: Vec<Address>,
}

impl ApprovedBuilders {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn builders(&self) -> &[Address] {
        &self.builders
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for ApprovedBuilders {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.approved_builders"])?;
        let builders = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/{index}");
                match value {
                    Value::String(text) => address_from_str(text, &path),
                    Value::Object(object) => require_address(object, &path, "address")
                        .or_else(|_| require_address(object, &path, "user")),
                    _ => Err(malformed(&path, "expected builder address")),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { builders })
    }
}

pub fn parse_approved_builders(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, ApprovedBuilders), InfoError> {
    parse_family(
        "official.info.approved_builders",
        raw,
        context,
        APPROVED_BUILDER_KNOWN_FIELDS,
        &[],
    )
}
