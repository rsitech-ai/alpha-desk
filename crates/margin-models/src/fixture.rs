use serde::{Deserialize, Serialize};

use crate::evaluate;
use crate::model::{MarginAssessment, MarginError, MarginInput};

pub const MARGIN_FIXTURE_SCHEMA: &str = "hl.margin.fixture.v1";
pub const SYNTHETIC_UNASSESSED: &str = "synthetic_unassessed";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MarginFixtureError {
    #[error("margin fixture decode failed: {0}")]
    Decode(String),
    #[error("margin fixture is unqualified-only: {0}")]
    Qualification(String),
    #[error("margin fixture expected state mismatch: {0}")]
    ExpectedMismatch(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarginFixture {
    pub schema: String,
    pub id: String,
    pub source_qualification: String,
    pub stage_1_qualified: bool,
    pub stage_2_qualified: bool,
    pub input: MarginInput,
    pub expected: MarginFixtureExpected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum MarginFixtureExpected {
    Assessment { assessment: MarginAssessment },
    Error { code: String },
}

pub fn parse_margin_fixture(json: &str) -> Result<MarginFixture, MarginFixtureError> {
    serde_json::from_str(json).map_err(|error| MarginFixtureError::Decode(error.to_string()))
}

pub fn assert_margin_fixture(fixture: &MarginFixture) -> Result<(), MarginFixtureError> {
    if fixture.schema != MARGIN_FIXTURE_SCHEMA {
        return Err(MarginFixtureError::Decode(format!(
            "unsupported schema {}",
            fixture.schema
        )));
    }
    if fixture.source_qualification != SYNTHETIC_UNASSESSED
        || fixture.stage_1_qualified
        || fixture.stage_2_qualified
    {
        return Err(MarginFixtureError::Qualification(
            "fixtures must remain synthetic_unassessed with Stage 1/2 false".to_owned(),
        ));
    }
    match (&fixture.expected, evaluate(&fixture.input)) {
        (MarginFixtureExpected::Assessment { assessment }, Ok(found)) => {
            if found == *assessment {
                Ok(())
            } else {
                Err(MarginFixtureError::ExpectedMismatch(
                    "assessment mismatch".to_owned(),
                ))
            }
        }
        (MarginFixtureExpected::Error { code }, Err(error)) => {
            if error_code(&error) == code.as_str() {
                Ok(())
            } else {
                Err(MarginFixtureError::ExpectedMismatch(format!(
                    "error {error:?} != {code}"
                )))
            }
        }
        (MarginFixtureExpected::Assessment { .. }, Err(error)) => Err(
            MarginFixtureError::ExpectedMismatch(format!("expected assessment, got {error:?}")),
        ),
        (MarginFixtureExpected::Error { code }, Ok(_)) => Err(
            MarginFixtureError::ExpectedMismatch(format!("expected error {code}")),
        ),
    }
}

fn error_code(error: &MarginError) -> &'static str {
    match error {
        MarginError::UnsupportedVersion => "UnsupportedVersion",
        MarginError::MissingInput(_) => "MissingInput",
        MarginError::Calculation(_) => "Calculation",
    }
}
