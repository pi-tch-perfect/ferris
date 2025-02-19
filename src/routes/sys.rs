use axum::{http::StatusCode, response::IntoResponse, Json};
use local_ip_address::local_ip;
use serde::Serialize;
use tracing::debug;

#[derive(Serialize)]
struct ServerIpResponse {
    ip: String
}

pub async fn server_ip(
) -> Result<impl IntoResponse, StatusCode> {
    let my_local_ip = local_ip().unwrap();

    debug!("my local ip {:?}", my_local_ip);

    Ok((StatusCode::OK, Json(ServerIpResponse { ip: my_local_ip.to_string() })))
}