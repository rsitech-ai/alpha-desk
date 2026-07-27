#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use test_fixtures::{FixtureError, FixtureManifest};

const USAGE: &str =
    "usage: fixture-inspect generate-manifest --root <directory> | verify <manifest.toml>";

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{USAGE}")]
    Usage,
    #[error("argument is not valid UTF-8: {0:?}")]
    NonUtf8Argument(OsString),
    #[error("manifest path has no parent directory: {0}")]
    ManifestWithoutParent(PathBuf),
    #[error(transparent)]
    Fixture(#[from] FixtureError),
}

fn main() -> ExitCode {
    match run(std::env::args_os()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<(), CliError> {
    let arguments = arguments
        .into_iter()
        .map(|argument| argument.into_string().map_err(CliError::NonUtf8Argument))
        .collect::<Result<Vec<_>, _>>()?;

    match arguments.as_slice() {
        [_, command, flag, root] if command == "generate-manifest" && flag == "--root" => {
            let manifest = FixtureManifest::generate(root)?;
            let digest = manifest.write_atomic(root)?;
            println!("manifest-sha256:{digest}");
            Ok(())
        }
        [_, command, manifest_path] if command == "verify" => verify(manifest_path),
        _ => Err(CliError::Usage),
    }
}

fn verify(manifest_path: impl AsRef<Path>) -> Result<(), CliError> {
    let manifest_path = manifest_path.as_ref();
    let root = manifest_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| CliError::ManifestWithoutParent(manifest_path.to_path_buf()))?;
    let manifest = FixtureManifest::load(manifest_path)?;
    manifest.verify(root)?;
    for id in manifest.fixture_ids_sorted() {
        println!("fixture:{id}:ok");
    }
    println!("manifest:ok");
    Ok(())
}
