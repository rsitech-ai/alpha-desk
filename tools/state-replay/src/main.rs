#![forbid(unsafe_code)]

use std::{ffi::OsString, path::PathBuf, process::ExitCode};

use state_replay::{
    ArchiveRunConfig, FixtureRunConfig, FixtureRunError, MarketRunConfig, OrderRunConfig,
    TradeRunConfig, run_archive_e2e, run_fixture_e2e, run_market_e2e, run_order_e2e, run_trade_e2e,
};

const USAGE: &str = "usage: state-replay fixture-e2e --output PATH --blocks N --checkpoint-after N --iterations N\n       state-replay trade-e2e --output PATH --blocks N --checkpoint-after N --iterations N\n       state-replay order-e2e --output PATH --blocks N --checkpoint-after N --iterations N\n       state-replay market-e2e --output PATH --blocks N --checkpoint-after N --iterations N\n       state-replay archive-e2e --archive PATH --output PATH --chain ID --start-height N --end-height N --checkpoint-height N --iterations N";

fn main() -> ExitCode {
    match run(std::env::args_os().collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage) => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        Err(CliError::Run(error)) => {
            eprintln!("ERROR {}", error.reason_code());
            ExitCode::from(1)
        }
    }
}

fn run(arguments: Vec<OsString>) -> Result<(), CliError> {
    let mut arguments = arguments.into_iter();
    let _program = arguments.next().ok_or(CliError::Usage)?;
    let command = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or(CliError::Usage)?;
    match command.as_str() {
        "fixture-e2e" => run_fixture(arguments),
        "trade-e2e" => run_trade(arguments),
        "order-e2e" => run_order(arguments),
        "market-e2e" => run_market(arguments),
        "archive-e2e" => run_archive(arguments),
        _ => Err(CliError::Usage),
    }
}

fn run_fixture(arguments: impl Iterator<Item = OsString>) -> Result<(), CliError> {
    let (output, blocks, checkpoint_after, iterations) = parse_replay_arguments(arguments)?;
    let config = FixtureRunConfig::new(output, blocks, checkpoint_after, iterations);
    let _evidence = run_fixture_e2e(&config)?;
    println!(
        "PASS evidence_class=synthetic_fixture stage_2_qualified=false live_source_qualified=false"
    );
    Ok(())
}

fn run_trade(arguments: impl Iterator<Item = OsString>) -> Result<(), CliError> {
    let (output, blocks, checkpoint_after, iterations) = parse_replay_arguments(arguments)?;
    let config = TradeRunConfig::new(output, blocks, checkpoint_after, iterations);
    let _evidence = run_trade_e2e(&config)?;
    println!(
        "PASS evidence_class=synthetic_canonical_trade state_semantics=canonical_trade_facts_and_exact_participant_anchors stage_1_qualified=false stage_2_qualified=false live_source_qualified=false"
    );
    Ok(())
}

fn run_order(arguments: impl Iterator<Item = OsString>) -> Result<(), CliError> {
    let (output, blocks, checkpoint_after, iterations) = parse_replay_arguments(arguments)?;
    let config = OrderRunConfig::new(output, blocks, checkpoint_after, iterations);
    let _evidence = run_order_e2e(&config)?;
    println!(
        "PASS evidence_class=synthetic_canonical_order state_semantics=exact_order_lifecycle synthetic_order_contract_proven=true stage_1_qualified=false stage_2_qualified=false live_source_qualified=false deployed_source_qualified=false position_state_qualified=false margin_state_qualified=false execution_qualified=false"
    );
    Ok(())
}

fn run_market(arguments: impl Iterator<Item = OsString>) -> Result<(), CliError> {
    let (output, blocks, checkpoint_after, iterations) = parse_replay_arguments(arguments)?;
    let config = MarketRunConfig::new(output, blocks, checkpoint_after, iterations);
    let _evidence = run_market_e2e(&config)?;
    println!(
        "PASS evidence_class=synthetic_canonical_market state_semantics=exact_market_registry synthetic_market_contract_proven=true stage_1_qualified=false stage_2_qualified=false live_source_qualified=false deployed_source_qualified=false authoritative_metadata_qualified=false external_oracle_reconciliation_qualified=false account_state_qualified=false position_state_qualified=false margin_state_qualified=false book_state_qualified=false signal_state_qualified=false execution_qualified=false"
    );
    Ok(())
}

fn parse_replay_arguments(
    mut arguments: impl Iterator<Item = OsString>,
) -> Result<(PathBuf, u64, u64, u64), CliError> {
    let mut output = None;
    let mut blocks = None;
    let mut checkpoint_after = None;
    let mut iterations = None;
    while let Some(flag) = arguments.next() {
        let flag = flag.into_string().map_err(|_| CliError::Usage)?;
        let value = arguments.next().ok_or(CliError::Usage)?;
        match flag.as_str() {
            "--output" if output.is_none() => output = Some(PathBuf::from(value)),
            "--blocks" if blocks.is_none() => blocks = Some(parse_u64(value)?),
            "--checkpoint-after" if checkpoint_after.is_none() => {
                checkpoint_after = Some(parse_u64(value)?);
            }
            "--iterations" if iterations.is_none() => iterations = Some(parse_u64(value)?),
            _ => return Err(CliError::Usage),
        }
    }
    Ok((
        output.ok_or(CliError::Usage)?,
        blocks.ok_or(CliError::Usage)?,
        checkpoint_after.ok_or(CliError::Usage)?,
        iterations.ok_or(CliError::Usage)?,
    ))
}

fn run_archive(mut arguments: impl Iterator<Item = OsString>) -> Result<(), CliError> {
    let mut archive = None;
    let mut output = None;
    let mut chain = None;
    let mut start_height = None;
    let mut end_height = None;
    let mut checkpoint_height = None;
    let mut iterations = None;
    while let Some(flag) = arguments.next() {
        let flag = flag.into_string().map_err(|_| CliError::Usage)?;
        let value = arguments.next().ok_or(CliError::Usage)?;
        match flag.as_str() {
            "--archive" if archive.is_none() => archive = Some(PathBuf::from(value)),
            "--output" if output.is_none() => output = Some(PathBuf::from(value)),
            "--chain" if chain.is_none() => {
                chain = Some(value.into_string().map_err(|_| CliError::Usage)?);
            }
            "--start-height" if start_height.is_none() => {
                start_height = Some(parse_u64(value)?);
            }
            "--end-height" if end_height.is_none() => end_height = Some(parse_u64(value)?),
            "--checkpoint-height" if checkpoint_height.is_none() => {
                checkpoint_height = Some(parse_u64(value)?);
            }
            "--iterations" if iterations.is_none() => iterations = Some(parse_u64(value)?),
            _ => return Err(CliError::Usage),
        }
    }
    let config = ArchiveRunConfig::new(
        archive.ok_or(CliError::Usage)?,
        output.ok_or(CliError::Usage)?,
        chain.ok_or(CliError::Usage)?,
        start_height.ok_or(CliError::Usage)?,
        end_height.ok_or(CliError::Usage)?,
        checkpoint_height.ok_or(CliError::Usage)?,
        iterations.ok_or(CliError::Usage)?,
    );
    let _evidence = run_archive_e2e(&config)?;
    println!(
        "PASS evidence_class=operator_archive state_semantics=watermark_only stage_2_qualified=false live_source_qualified=false"
    );
    Ok(())
}

fn parse_u64(value: OsString) -> Result<u64, CliError> {
    value
        .into_string()
        .map_err(|_| CliError::Usage)?
        .parse()
        .map_err(|_| CliError::Usage)
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{USAGE}")]
    Usage,
    #[error(transparent)]
    Run(#[from] FixtureRunError),
}
