use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info};
use unidecode::unidecode;

use crate::globals;

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub id: String,
}

#[derive(Error, Debug)]
pub enum SearchError {
    #[error("Failed to execute youtube-dl: {0}")]
    ExecutionError(#[from] std::io::Error),
    #[error("Failed to parse JSON output: {0}")]
    JsonParseError(#[from] serde_json::Error),
    #[error("Missing required fields in response")]
    MissingFields,
}

pub struct YtSearcher {}

impl YtSearcher {
    pub async fn search(&self, query: &str) -> Result<Vec<SearchResult>, SearchError> {
        info!("searching yt-dlp for: {}", query);
        
        let num_results = 10;
        let search_query = format!("ytsearch{}:\"{}\"", num_results, unidecode(query));
        
        let args = [
            "-j",
            "--no-playlist",
            "--flat-playlist",
            "--match-filter",
            "!is_channel",
            &search_query,
        ];
        
        debug!("yt-dlp search command: {:?}", args.join(" "));


        let ytdlp_path = globals::get_binary_path("yt-dlp");
        debug!("Using yt-dlp from path: {}", ytdlp_path.display());

        let output = std::process::Command::new(ytdlp_path)
            .args(&args)
            .output()?;

        let output_str = String::from_utf8_lossy(&output.stdout);
        debug!("search results: {}", output_str);

        output_str
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let json: serde_json::Value = serde_json::from_str(line)?;
                
                let title = json.get("title")
                    .and_then(|v| v.as_str())
                    .ok_or(SearchError::MissingFields)?;
                    
                let url = json.get("url")
                    .and_then(|v| v.as_str())
                    .ok_or(SearchError::MissingFields)?;
                    
                let id = json.get("id")
                    .and_then(|v| v.as_str())
                    .ok_or(SearchError::MissingFields)?;

                Ok(SearchResult {
                    title: title.to_string(),
                    url: url.to_string(),
                    id: id.to_string(),
                })
            })
            .collect()
    }
}