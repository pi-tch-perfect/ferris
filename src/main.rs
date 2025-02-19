use std::{fs, path::PathBuf, process::Command};

use axum::serve;
use dotenv::dotenv;
use rust_embed::RustEmbed;
use tokio::net::TcpListener;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::{debug, error, info};
use tracing_subscriber::{fmt::format::FmtSpan, EnvFilter};

use crate::router::create_router_with_state;

mod actors;
mod globals;
mod router;
mod routes;
mod state;
mod utils;

#[derive(RustEmbed)]
#[folder = "embedded/"]
struct Asset;

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

fn update_ytdlp(config_dir: &PathBuf) -> Result<(), DependencyError> {
    let ytdlp_path = config_dir.join(if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    });

    // Always extract the embedded version first
    debug!("Extracting embedded yt-dlp");
    setup_binary("yt-dlp", config_dir)?;

    debug!(
        "Updating extracted yt-dlp at path: {}",
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

fn setup_binary(name: &str, config_dir: &PathBuf) -> Result<(), DependencyError> {
    let bin_path = config_dir.join(if cfg!(windows) {
        format!("{}.exe", name)
    } else {
        name.to_string()
    });

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

#[tokio::main]
async fn main() -> Result<(), DependencyError> {
    // Initialize environment
    dotenv().ok();

    // Initialize logging with timestamps and target info
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .or_else(|_| EnvFilter::try_new("ferris=debug,tower_http=debug"))
                .unwrap(),
        )
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true)
        .with_span_events(FmtSpan::CLOSE)
        .init();

    info!("Starting ferris server");
    debug!("Initializing configuration and directories");

    // Setup config directory and binaries
    let config_dir = dirs::config_dir()
        .ok_or_else(|| {
            error!("Failed to determine config directory");
            DependencyError::NoConfigDir
        })?
        .join("pi-tchperfect");

    globals::init_config_dir(config_dir.clone());

    debug!("Creating config directory at: {}", config_dir.display());
    fs::create_dir_all(&config_dir).map_err(|e| {
        error!("Failed to create config directory: {}", e);
        DependencyError::Io(e)
    })?;

    info!("Setting up required binaries");
    setup_binary("ffmpeg", &config_dir)?;
    update_ytdlp(&config_dir)?; // This will extract embedded yt-dlp and then update it

    // Setup CORS
    debug!("Configuring CORS");
    let cors_layer = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Create and configure app
    info!("Creating router and configuring middleware");
    let app = create_router_with_state()
        .await
        .layer(cors_layer)
        .layer(TraceLayer::new_for_http());

    // Start server
    let addr = "0.0.0.0:8000";
    info!("Starting server on {}", addr);
    let listener = TcpListener::bind(addr).await.unwrap();

    info!("Server is ready to accept connections");
    match serve(listener, app).await {
        Ok(_) => info!("Server shutdown gracefully"),
        Err(e) => error!("Server error: {}", e),
    }

    Ok(())
}
