mod build_support;

fn main() {
    let manifest_dir = match std::env::var_os("CARGO_MANIFEST_DIR") {
        Some(value) => std::path::PathBuf::from(value),
        None => {
            eprintln!("build metadata error: CARGO_MANIFEST_DIR is unavailable");
            std::process::exit(1);
        }
    };
    if let Err(error) = build_support::emit_build_metadata(&manifest_dir) {
        eprintln!("build metadata error: {error}");
        std::process::exit(1);
    }
}
