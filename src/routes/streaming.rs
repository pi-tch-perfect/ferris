use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::path::PathBuf;
use tokio::{fs::File, io::AsyncReadExt};

#[derive(Debug)]
pub struct FileError(std::io::Error);

impl IntoResponse for FileError {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("File error: {}", self.0),
        )
            .into_response()
    }
}

pub async fn serve_dash_file(Path((song_name, file)): Path<(String, String)>) -> Result<Response, FileError> {
    let path = PathBuf::from("./")
        .join("assets")
        .join(&song_name)
        .join (&file);

    let mut file = File::open(&path).await.map_err(FileError)?;
    let mut contents = vec![];
    file.read_to_end(&mut contents).await.map_err(FileError)?;

    let content_type = match path.extension().and_then(|ext| ext.to_str()) {
        Some("mpd") => "application/dash+xml",
        Some("m4s") => "video/iso.segment",
        Some("mp4") => "video/mp4",
        _ => "application/octet-stream",
    };

    Ok((StatusCode::OK, [("Content-Type", content_type)], contents).into_response())
}

