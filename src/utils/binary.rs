use std::{fs, path::PathBuf, process::Command};
use rust_embed::RustEmbed;
use tracing::{debug, error, info};

#[derive(RustEmbed)]
#[folder = "embedded/"]
pub struct Asset;

#[derive(Debug, Clone, Copy)]
pub enum Binary {
    Ytdlp,
    Ffmpeg,
}

impl Binary {
    fn name(&self) -> &'static str {
        match self {
            Binary::Ytdlp => "yt-dlp",
            Binary::Ffmpeg => "ffmpeg",
        }
    }

    fn get_path(&self, config_dir: &PathBuf) -> PathBuf {
        config_dir.join(if cfg!(windows) {
            format!("{}.exe", self.name())
        } else {
            self.name().to_string()
        })
    }
}

#[derive(thiserror::Error, Debug)]
pub enum DependencyError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to determine config directory")]
    NoConfigDir,

    #[error("Could not find embedded binary: {0}")]
    MissingBinary(String),

    #[error("Failed to update yt-dlp")]
    YtDlpUpdateFailed,

    #[error("Command failed: {0}")]
    CommandFailed(String),
}

pub fn update_ytdlp(config_dir: &PathBuf) -> Result<(), DependencyError> {
    let ytdlp_path = Binary::Ytdlp.get_path(config_dir);

    debug!(
        "Updating yt-dlp at path: {}",
        ytdlp_path.display()
    );
    let status = Command::new(&ytdlp_path).arg("-U").status().map_err(|e| {
        error!("Failed to execute yt-dlp update: {}", e);
        DependencyError::CommandFailed(e.to_string())
    })?;

    if !status.success() {
        error!("yt-dlp update process failed with status: {}", status);
        return Err(DependencyError::YtDlpUpdateFailed);
    }

    info!("yt-dlp successfully updated");
    Ok(())
}

pub fn setup_binary(binary: Binary, config_dir: &PathBuf) -> Result<(), DependencyError> {
    let name = binary.name();
    let bin_path = binary.get_path(config_dir);

    debug!(
        "Setting up binary '{}' at path: {}",
        name,
        bin_path.display()
    );

    let asset_name = if cfg!(windows) {
        format!("{}.exe", name)
    } else {
        name.to_string()
    };

    debug!("Extracting embedded asset: {}", asset_name);
    let binary = Asset::get(&asset_name).ok_or_else(|| {
        error!("Could not find embedded binary: {}", asset_name);
        DependencyError::MissingBinary(asset_name.clone())
    })?;

    // Remove existing binary if it exists
    if bin_path.exists() {
        debug!("Removing existing binary at: {}", bin_path.display());
        fs::remove_file(&bin_path).map_err(|e| {
            error!("Failed to remove existing binary: {}", e);
            DependencyError::Io(e)
        })?;
    }

    debug!("Writing binary data to: {}", bin_path.display());
    fs::write(&bin_path, binary.data).map_err(|e| {
        error!("Failed to write binary data: {}", e);
        DependencyError::Io(e)
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        debug!("Setting executable permissions on Unix system");
        let mut perms = fs::metadata(&bin_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin_path, perms).map_err(|e| {
            error!("Failed to set executable permissions: {}", e);
            DependencyError::Io(e)
        })?;
    }

    info!("Successfully set up binary: {}", name);
    Ok(())
}