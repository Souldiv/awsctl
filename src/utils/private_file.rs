use anyhow::{anyhow, Context, Result};
use std::fs::{DirBuilder, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub const PRIVATE_FILE_MODE: u32 = 0o600;
pub const PRIVATE_DIR_MODE: u32 = 0o700;

pub fn replace_private_file(path: &Path, contents: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("The path {} has no parent directory.", path.display()))?;

    DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIR_MODE)
        .create(parent)
        .with_context(|| format!("Unable to create the directory {}.", parent.display()))?;

    let temporary_path = temporary_path(path);
    let written = fill_private_file(&temporary_path, contents).and_then(|()| {
        std::fs::rename(&temporary_path, path)
            .with_context(|| format!("Unable to move {} into place.", path.display()))
    });

    if written.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }

    written
}

fn temporary_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from("file"));

    path.with_file_name(format!(".{}.{}.tmp", file_name, std::process::id()))
}

fn fill_private_file(path: &Path, contents: &str) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Unable to clear the temporary file {}.", path.display()))
        }
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(PRIVATE_FILE_MODE)
        .open(path)
        .with_context(|| format!("Unable to create {}.", path.display()))?;

    file.write_all(contents.as_bytes())
        .with_context(|| format!("Unable to write {}.", path.display()))?;
    file.sync_all()
        .with_context(|| format!("Unable to flush {}.", path.display()))?;

    Ok(())
}
