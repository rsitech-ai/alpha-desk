use std::{ffi::OsStr, path::PathBuf, process::ExitCode};

fn run() -> Result<&'static str, String> {
    let arguments = std::env::args_os().collect::<Vec<_>>();
    match arguments.as_slice() {
        [_, command, descriptor_flag, descriptor, rust_flag, rust_output]
            if command == OsStr::new("contracts")
                && descriptor_flag == OsStr::new("--descriptor")
                && rust_flag == OsStr::new("--rust-out") =>
        {
            api_contracts::export_contract_artifacts(
                PathBuf::from(descriptor),
                PathBuf::from(rust_output),
            )
            .map_err(|error| format!("contract export failed: {error}"))?;
            Ok("contracts")
        }
        [_, command, root_flag, schema_root, output_flag, output]
            if command == OsStr::new("material")
                && root_flag == OsStr::new("--schema-root")
                && output_flag == OsStr::new("--output") =>
        {
            api_contracts::write_schema_material(
                PathBuf::from(schema_root),
                PathBuf::from(output),
            )
            .map_err(|error| format!("material generation failed: {error}"))?;
            Ok("material")
        }
        _ => Err(
            "usage: schema-generate contracts --descriptor <new-file> --rust-out <empty-directory> | material --schema-root <directory> --output <new-file>"
                .to_owned(),
        ),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(kind) => {
            println!("schema-generation:{kind}:ok");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("schema-generation:error {error}");
            ExitCode::from(2)
        }
    }
}
