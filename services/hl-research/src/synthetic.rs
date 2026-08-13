use std::path::Path;

use execution_sim::{SimulationRequest, run};
use model_runtime::{
    ModelRegistry, ModelState, SignedBundle, TransitionEvidence, score_research_bundle,
};
use serde::Deserialize;

use crate::error::ResearchError;
use crate::experiment::{ExperimentManifest, ExperimentRegistry, ExperimentStatus};
use crate::report::ResearchReport;

#[derive(Debug, Deserialize)]
pub struct SyntheticFixture {
    experiment: ExperimentManifest,
    simulation: SimulationRequest,
    #[serde(default)]
    model_features: Option<ModelFeatureInput>,
}

#[derive(Debug, Deserialize)]
struct ModelFeatureInput {
    names: Vec<String>,
    values: Vec<String>,
}

pub fn run_synthetic_bytes(
    bytes: &[u8],
    bundle_dir: Option<&Path>,
    approved_key: Option<[u8; 32]>,
) -> Result<ResearchReport, ResearchError> {
    let fixture: SyntheticFixture =
        serde_json::from_slice(bytes).map_err(|_| ResearchError::InvalidFixture)?;
    run_synthetic_fixture(fixture, bundle_dir, approved_key)
}

pub fn run_synthetic_fixture(
    fixture: SyntheticFixture,
    bundle_dir: Option<&Path>,
    approved_key: Option<[u8; 32]>,
) -> Result<ResearchReport, ResearchError> {
    let mut registry = ExperimentRegistry::new();
    let (status, experiment_id) = match registry.submit(fixture.experiment.clone()) {
        Ok(record) => (record.status, record.experiment_id.to_string()),
        Err(ResearchError::IncompleteManifest { .. }) => (
            ExperimentStatus::Exploratory,
            fixture
                .experiment
                .experiment_id()
                .map(|id| id.to_string())
                .unwrap_or_else(|_| "exploratory".to_owned()),
        ),
        Err(error) => return Err(error),
    };

    let simulation = fixture.simulation;
    simulation.validate().map_err(ResearchError::from)?;
    let result = run(&simulation)?;

    let model_score = match bundle_dir {
        Some(path) => {
            let key = approved_key.ok_or(ResearchError::InvalidFixture)?;
            let bundle = SignedBundle::load_dir(path, &[key], &model_runtime::Ed25519Verifier)?;
            let mut models = ModelRegistry::new();
            let version = models.register(&bundle)?;
            models.advance(
                &version,
                ModelState::ResearchPassed,
                TransitionEvidence::SyntheticResearch,
            )?;
            let input = fixture
                .model_features
                .ok_or(ResearchError::InvalidFixture)?;
            let values = input
                .values
                .iter()
                .map(|value| value.parse().map_err(|_| ResearchError::InvalidFixture))
                .collect::<Result<Vec<_>, _>>()?;
            let score = score_research_bundle(&models, &version, &bundle, &input.names, &values)?;
            Some(score.value().to_string())
        }
        None => None,
    };

    Ok(ResearchReport::from_synthetic(
        status,
        experiment_id,
        &result,
        model_score,
    ))
}
