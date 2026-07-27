#![forbid(unsafe_code)]

use schema_check::{check_file_descriptor_sets, read_descriptor_file};
use std::{env, path::PathBuf, process::ExitCode};

fn run() -> Result<(), String> {
    let mut arguments = env::args_os();
    let _program = arguments.next();
    let command = arguments.next();
    let baseline = arguments.next();
    let current = arguments.next();
    let extra = arguments.next();

    if command.as_deref() != Some(std::ffi::OsStr::new("check"))
        || baseline.is_none()
        || current.is_none()
        || extra.is_some()
    {
        return Err(
            "usage: schema-check check <baseline-descriptor.pb> <current-descriptor.pb>".to_owned(),
        );
    }

    let baseline_path = PathBuf::from(baseline.ok_or_else(|| "missing baseline path".to_owned())?);
    let current_path = PathBuf::from(current.ok_or_else(|| "missing current path".to_owned())?);
    let baseline_descriptor =
        read_descriptor_file("baseline", &baseline_path).map_err(|error| error.to_string())?;
    let current_descriptor =
        read_descriptor_file("current", &current_path).map_err(|error| error.to_string())?;
    check_file_descriptor_sets(&baseline_descriptor, &current_descriptor)
        .map_err(|error| error.to_string())?;
    println!("schema-compatibility:compatible");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("schema-compatibility:incompatible: {error}");
            ExitCode::from(2)
        }
    }
}
