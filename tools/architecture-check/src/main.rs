use std::ffi::OsStr;
use std::path::PathBuf;

fn main() {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let metadata_path = match arguments.as_slice() {
        [command] if command == OsStr::new("check") => None,
        [command, flag, path]
            if command == OsStr::new("check") && flag == OsStr::new("--metadata") =>
        {
            Some(PathBuf::from(path))
        }
        _ => {
            eprintln!("usage: architecture-check check [--metadata <path>]");
            std::process::exit(2);
        }
    };

    let metadata = match architecture_check::load_metadata(metadata_path.as_deref()) {
        Ok(metadata) => metadata,
        Err(error) => {
            eprintln!("metadata-error: {error}");
            std::process::exit(1);
        }
    };
    let diagnostics = match architecture_check::check(&metadata) {
        Ok(diagnostics) => diagnostics,
        Err(error) => {
            eprintln!("metadata-error: {error}");
            std::process::exit(1);
        }
    };

    if diagnostics.is_empty() {
        return;
    }
    for diagnostic in diagnostics {
        eprintln!("{diagnostic}");
    }
    std::process::exit(1);
}
