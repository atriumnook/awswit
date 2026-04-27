use std::path::PathBuf;

/// Get the awswit home directory (~/.awswit)
pub fn awswit_home_dir() -> Result<PathBuf, std::io::Error> {
    let home = dirs::home_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Could not determine home directory",
        )
    })?;
    Ok(home.join(".awswit"))
}
