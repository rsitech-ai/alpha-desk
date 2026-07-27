use std::path::Path;

fn main() {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let metadata_path = match arguments.as_slice() {
        [command] if command == "check" => None,
        [command, flag, path] if command == "check" && flag == "--metadata" => {
            Some(Path::new(path))
        }
        _ => {
            eprintln!("usage: architecture-check check [--metadata <path>]");
            std::process::exit(2);
        }
    };

    let metadata = match architecture_check::load_metadata(metadata_path) {
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
