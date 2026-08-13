#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use orderbook::{BookHealth, BookReplayReport, parse_book_fixture, replay_book_fixture};

const USAGE: &str = "usage: book-inspect replay <fixture.json>";

fn main() -> ExitCode {
    match run(std::env::args_os()) {
        Ok(report) => {
            println!(
                "PASS id={} health={} sequence={} orders={}",
                report.id,
                health_label(&report.health),
                report.sequence,
                report.active_order_count
            );
            ExitCode::SUCCESS
        }
        Err(CliError::Usage | CliError::NonUtf8Argument(_)) => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<BookReplayReport, CliError> {
    let arguments = arguments
        .into_iter()
        .map(|argument| argument.into_string().map_err(CliError::NonUtf8Argument))
        .collect::<Result<Vec<_>, _>>()?;
    match arguments.as_slice() {
        [_, command, path] if command == "replay" => replay(Path::new(path)),
        _ => Err(CliError::Usage),
    }
}

fn replay(path: &Path) -> Result<BookReplayReport, CliError> {
    let json = fs::read_to_string(path).map_err(|_| CliError::Read)?;
    let fixture = parse_book_fixture(&json)?;
    Ok(replay_book_fixture(&fixture)?)
}

fn health_label(health: &BookHealth) -> &'static str {
    match health {
        BookHealth::Healthy => "healthy",
        BookHealth::AwaitingSnapshot { .. } => "awaiting_snapshot",
        BookHealth::Red { .. } => "red",
    }
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{USAGE}")]
    Usage,
    #[error("argument is not valid UTF-8")]
    NonUtf8Argument(OsString),
    #[error("book fixture could not be read")]
    Read,
    #[error(transparent)]
    Fixture(#[from] orderbook::BookFixtureError),
}
