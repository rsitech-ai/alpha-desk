use std::process::ExitCode;

use telemetry::BuildProvenance;

fn main() -> ExitCode {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let command = arguments.next();
    if command.as_deref() != Some(std::ffi::OsStr::new("print")) || arguments.next().is_some() {
        eprintln!("usage: build-info print");
        return ExitCode::FAILURE;
    }

    match BuildProvenance::current().and_then(|build| build.to_json()) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("build-info failed: {error}");
            ExitCode::FAILURE
        }
    }
}
