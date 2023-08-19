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
use xrtc_core::message::XrtcMessage;
use xrtc_core::transport::SharedConnection;
use xrtc_core::transport::SharedTransport;
use xrtc_swarm::Backend;
use xrtc_swarm::Swarm;

type ServiceState<T, B> = Arc<Swarm<T, B>>;

#[derive(Deserialize, Serialize)]
pub struct Connect {
    pub cid: String,
    pub endpoint: String,
}

#[derive(Deserialize, Serialize)]
struct Handshake {
    cid: String,
    sdp: String,
}

#[derive(Deserialize, Serialize)]
struct SendMessage {
    cid: String,
    message: XrtcMessage,
}

#[derive(Deserialize, Serialize, Debug)]
struct ErrorMessage {
    error: String,
}

#[derive(thiserror::Error, Debug)]
enum ServiceError {
    #[error("Xrtc swarm error: {0}")]
    Swarm(#[from] Box<dyn std::error::Error + Send + Sync>),
    #[error("Peer request error: {0:?}")]
    PeerRequestError(#[from] reqwest::Error),
    #[error("Peer response error: {0:?}")]
    PeerResponseError(ErrorMessage),
    #[error("Serde json error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("Connection not found: {cid}")]
    ConnectionNotFound { cid: String },
}

impl ServiceError {
    fn swarm_error<E>(e: E) -> Self
    where E: std::error::Error + Send + Sync + 'static {
        ServiceError::Swarm(Box::new(e))
    }
}

impl IntoResponse for ServiceError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            ServiceError::Swarm(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
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

pub async fn run_http_service<T, B>(xrtc_server: Arc<Swarm<T, B>>, service_address: &str)
where
    T: SharedTransport,
    B: Backend + Send + Sync + 'static,
{
    let state = xrtc_server.clone();
    let router = Router::new()
        .route("/connect", post(connect))
        .route("/answer_offer", post(answer_offer))
        .with_state(state);
    Server::bind(&service_address.parse().unwrap())
        .serve(router.into_make_service())
        .await
        .unwrap();
}

async fn connect<T, B>(
    State(state): State<ServiceState<T, B>>,
    Json(payload): Json<Connect>,
) -> Result<(), ServiceError>
where
    T: SharedTransport,
    B: Backend,
{
    state
        .new_connection(&payload.cid)
        .await
        .map_err(ServiceError::swarm_error)?;

    let Some(conn) = state.get_connection(&payload.cid) else {
        return Err(ServiceError::ConnectionNotFound { cid: payload.cid });
    };

    let offer = conn
        .webrtc_create_offer()
        .await
        .map_err(ServiceError::swarm_error)?;

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

    let answer = serde_json::from_str(&handshake.sdp).map_err(ServiceError::swarm_error)?;
    conn.webrtc_accept_answer(answer)
        .await
        .map_err(ServiceError::swarm_error)?;

    Ok(())
}

async fn answer_offer<T, B>(
    State(state): State<ServiceState<T, B>>,
    Json(payload): Json<Handshake>,
) -> Result<Json<Handshake>, ServiceError>
where
    T: SharedTransport,
    B: Backend + Send + Sync,
{
    let offer = serde_json::from_str(&payload.sdp)?;

    state
        .new_connection(&payload.cid)
        .await
        .map_err(ServiceError::swarm_error)?;

    let Some(conn) = state.get_connection(&payload.cid) else {
        return Err(ServiceError::ConnectionNotFound { cid: payload.cid });
    };

    let answer = conn
        .webrtc_answer_offer(offer)
        .await
        .map_err(ServiceError::swarm_error)?;
    let response = Handshake {
        cid: payload.cid.clone(),
        sdp: serde_json::to_string(&answer)?,
    };

    Ok(Json(response))
}
