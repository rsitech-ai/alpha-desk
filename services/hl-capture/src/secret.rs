use std::fs;
use std::path::Path;

const MAX_SECRET_BYTES: u64 = 16_384;

pub(crate) fn read_protected_secret(path: &Path) -> Result<String, std::io::Error> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_SECRET_BYTES
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "unsafe secret file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "secret file permissions are too broad",
            ));
        }
    }
    let bytes = fs::read(path)?;
    let value = String::from_utf8(bytes)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "secret UTF-8"))?;
    let value = value.strip_suffix('\n').unwrap_or(&value);
    let value = value.strip_suffix('\r').unwrap_or(value);
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid secret",
        ));
    }
    Ok(value.to_owned())
}
