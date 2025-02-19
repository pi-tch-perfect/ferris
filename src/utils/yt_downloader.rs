use std::process::Command;
use thiserror::Error;
use tracing::debug;

use crate::globals;

#[derive(Error, Debug)]
pub enum VideoProcessError {
    #[error("YouTube download failed: {0}")]
    DownloadError(String),
    #[error("Failed to process filename: {0}")]
    FilenameError(String),
    #[error("Pitch shift processing failed: {0}")]
    PitchShiftError(String),
    #[error("Video extraction processing failed: {0}")]
    VideoExtractError(String),
    #[error("Command execution failed: {0}")]
    CommandError(#[from] std::io::Error),
    #[error("Failed to parse duration: {0}")]
    DurationParseError(String),
}

#[derive(Debug)]
pub struct VideoMetadata {
    pub directory: String,
    pub filename: String,
    pub extension: String,
    pub duration_seconds: f64,
}

#[derive(Clone)]
pub struct YtDownloader {}

impl YtDownloader {
    pub async fn download(
        &self,
        yt_link: &str,
        base_dir: &str,
        file_name: &str,
    ) -> Result<VideoMetadata, VideoProcessError> {
        let ffmpeg_path = globals::get_binary_path("ffmpeg");

        let args = vec![
            "-f".to_string(),
            "bestvideo[height<=720][vcodec^=avc1]+bestaudio".to_string(),
            "-o".to_string(),
            format!("{}/{}/{}.%(ext)s", base_dir, file_name, file_name),
            "--merge-output-format".to_string(),
            "mp4".to_string(),
            "--restrict-filenames".to_string(),
            "--print".to_string(),
            "filename,duration".to_string(),  // Print both filename and duration
            "--no-simulate".to_string(),
            "--ffmpeg-location".to_string(),
            ffmpeg_path.to_string_lossy().to_string(),
            "--".to_string(),
            format!("{}", yt_link.to_string()),
        ];

        debug!("yt-dlp command: {:?}", args);

        let ytdlp_path = globals::get_binary_path("yt-dlp");
        debug!("Using yt-dlp from path: {}", ytdlp_path.display());

        let output = Command::new(ytdlp_path)
            .args(&args)
            .output()
            .map_err(VideoProcessError::CommandError)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VideoProcessError::DownloadError(stderr.to_string()));
        }

        let parsed = self.parse_output(&output.stdout);
        debug!("parseed {:?}", parsed);

        parsed
    }

    fn parse_output(&self, output: &[u8]) -> Result<VideoMetadata, VideoProcessError> {
        let output_str = String::from_utf8(output.to_vec())
            .map_err(|e| VideoProcessError::FilenameError(e.to_string()))?;
        
        // Split the output into lines
        let lines: Vec<&str> = output_str.lines().collect();
        if lines.len() != 2 {
            return Err(VideoProcessError::FilenameError(
                "Expected filename and duration output".to_string(),
            ));
        }

        let filename = lines[0].trim();
        let duration_str = lines[1].trim();

        // Parse the duration (convert from string to f64)
        let duration_seconds = duration_str
            .parse::<f64>()
            .map_err(|e| VideoProcessError::DurationParseError(e.to_string()))?;

        // Split the path into components
        let path_parts: Vec<&str> = filename.rsplitn(2, '/').collect();
        if path_parts.len() != 2 {
            return Err(VideoProcessError::FilenameError("Invalid path format".to_string()));
        }

        let full_filename = path_parts[0];
        let directory = path_parts[1];

        // Split the filename and extension
        let (name, ext) = full_filename
            .rsplit_once('.')
            .ok_or_else(|| VideoProcessError::FilenameError("Invalid filename format".to_string()))?;

        Ok(VideoMetadata {
            directory: directory.to_string(),
            filename: name.to_string(),
            extension: ext.to_string(),
            duration_seconds,
        })
    }
}