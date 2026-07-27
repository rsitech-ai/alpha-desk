use std::{
    env,
    error::Error,
    fs, io,
    path::{Path, PathBuf},
};

fn workspace_root(manifest_dir: &Path) -> Result<&Path, io::Error> {
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| io::Error::other("api-contracts must live under <workspace>/crates"))
}

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let root = workspace_root(&manifest_dir)?;
    let schemas = root.join("schemas/proto");
    let protos = [
        schemas.join("common/v1/types.proto"),
        schemas.join("canonical/v1/events.proto"),
        schemas.join("stream/v1/envelope.proto"),
    ];

    for proto in &protos {
        println!("cargo:rerun-if-changed={}", proto.display());
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let generated_descriptor = out_dir.join("alpha-desk-v1.pb");
    let current_descriptor = root.join("target/schema/current.pb");
    let current_parent = current_descriptor
        .parent()
        .ok_or_else(|| io::Error::other("current descriptor path has no parent"))?;
    fs::create_dir_all(current_parent)?;

    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    let mut prost_config = tonic_prost_build::Config::new();
    prost_config
        .protoc_executable(protoc)
        .file_descriptor_set_path(&generated_descriptor)
        .skip_source_info();

    // Tonic 0.14 moved Prost-backed compilation out of tonic-build and into
    // tonic-prost-build. Keep this correction explicit so the removed
    // tonic_build::compile_protos path is not accidentally restored.
    tonic_prost_build::configure()
        .build_client(false)
        .build_server(false)
        .compile_with_config(prost_config, &protos, &[schemas])?;

    fs::copy(&generated_descriptor, &current_descriptor)?;
    Ok(())
}
