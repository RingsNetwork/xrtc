use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::post;
use axum::Json;
use axum::Router;
use axum::Server;
use serde::Deserialize;
use serde::Serialize;

use crate::error::Error;
use crate::ConnectionId;
use crate::XrtcMessage;
use crate::XrtcServer;

#[derive(Clone)]
struct ServiceState {
    xrtc_server: Arc<XrtcServer>,
}

#[derive(Deserialize, Serialize)]
pub struct Connect {
    pub cid: ConnectionId,
    pub endpoint: String,
}

#[derive(Deserialize, Serialize)]
struct Handshake {
    cid: ConnectionId,
    sdp: String,
}

#[derive(Deserialize, Serialize)]
struct SendMessage {
    cid: ConnectionId,
    message: XrtcMessage,
}

#[derive(Deserialize, Serialize, Debug)]
struct ErrorMessage {
    error: String,
}

#[derive(thiserror::Error, Debug)]
enum ServiceError {
    #[error("Xrtc server error: {0}")]
    XrtcServer(#[from] Error),
    #[error("Peer request error: {0:?}")]
    PeerRequestError(#[from] reqwest::Error),
    #[error("Peer response error: {0:?}")]
    PeerResponseError(ErrorMessage),
    #[error("Serde json error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("Connection not found: {conn_id}")]
    ConnectionNotFound { conn_id: String },
}

impl IntoResponse for ServiceError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            ServiceError::XrtcServer(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            ServiceError::PeerRequestError(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            ServiceError::PeerResponseError(e) => (
                StatusCode::BAD_REQUEST,
                format!("Peer response error: {}", e.error),
            ),
            ServiceError::SerdeJson(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            e @ ServiceError::ConnectionNotFound { .. } => (StatusCode::NOT_FOUND, e.to_string()),
        };

        tracing::error!("ServiceError: {status} - {error_message}");

        let body = Json(ErrorMessage {
            error: error_message,
        });

        (status, body).into_response()
    }
}

pub async fn run_http_service(xrtc_server: Arc<XrtcServer>, service_address: &str) {
    let state = ServiceState { xrtc_server };
    let router = Router::new()
        .route("/connect", post(connect))
        .route("/answer_offer", post(answer_offer))
        .with_state(state);
    Server::bind(&service_address.parse().unwrap())
        .serve(router.into_make_service())
        .await
        .unwrap();
}

async fn connect(
    State(state): State<ServiceState>,
    Json(payload): Json<Connect>,
) -> Result<(), ServiceError> {
    state
        .xrtc_server
        .new_connection(payload.cid.clone())
        .await?;
    let Some(conn) = state.xrtc_server.connections.get(&payload.cid) else {
        return Err(ServiceError::ConnectionNotFound{conn_id: payload.cid})
    };

    let offer = conn.webrtc_create_offer().await?;

    let handshake_resp = reqwest::Client::new()
        .post(format!("{}/answer_offer", payload.endpoint))
        .json(&Handshake {
            cid: payload.cid.clone(),
            sdp: serde_json::to_string(&offer)?,
        })
        .send()
        .await?;

    if !handshake_resp.status().is_success() {
        let error_message = handshake_resp.json::<ErrorMessage>().await?;
        return Err(ServiceError::PeerResponseError(error_message));
    }

    let handshake = handshake_resp.json::<Handshake>().await?;

    let answer = serde_json::from_str(&handshake.sdp)?;
    conn.webrtc_accept_answer(answer).await?;

    Ok(())
}

async fn answer_offer(
    State(state): State<ServiceState>,
    Json(payload): Json<Handshake>,
) -> Result<Json<Handshake>, ServiceError> {
    let offer = serde_json::from_str(&payload.sdp)?;

    state
        .xrtc_server
        .new_connection(payload.cid.clone())
        .await?;
    let Some(conn) = state.xrtc_server.connections.get(&payload.cid) else {
        return Err(ServiceError::ConnectionNotFound{conn_id: payload.cid})
    };

    let answer = conn.webrtc_answer_offer(offer).await?;
    let response = Handshake {
        cid: payload.cid.clone(),
        sdp: serde_json::to_string(&answer)?,
    };

    Ok(Json(response))
}
