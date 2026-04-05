use crate::channel::{self, ChangeEvent, ChangeEventSender};
use crate::notification;
use crate::subscription::{ChannelType, Subscription, SubscriptionRegistry};
use axum::extract::ws;
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::response::Sse;
use axum::response::sse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

/// Configuration for the CDC notification server.
#[derive(Clone)]
pub struct CdcConfig {
    /// Port for the CDC server to bind to.
    pub bind_port: u16,
    /// URL of the main SPARQL server (for topic URLs and Link header).
    pub main_server_url: String,
    /// Max queued notifications per subscriber.
    pub buffer_size: usize,
    /// Batching time window.
    pub batch_window: Duration,
}

/// The CDC notification server.
pub struct CdcServer {
    sender: ChangeEventSender,
    config: CdcConfig,
}

#[derive(Clone)]
struct AppState {
    sender: ChangeEventSender,
    registry: Arc<SubscriptionRegistry>,
    config: CdcConfig,
}

impl CdcServer {
    /// Create a new CDC server. Returns the sender (for the main server) and the server itself.
    pub fn new(config: CdcConfig) -> (ChangeEventSender, Self) {
        let (sender, _rx) = channel::new_broadcast(config.buffer_size);
        let sender_clone = sender.clone();
        (sender_clone, Self { sender, config })
    }

    /// Run the CDC server. This is an async function that runs until the server shuts down.
    ///
    /// # Errors
    ///
    /// Returns an error if the TCP listener cannot bind or the server encounters a fatal error.
    pub async fn run(self) -> Result<(), std::io::Error> {
        let state = AppState {
            sender: self.sender,
            registry: Arc::new(SubscriptionRegistry::new()),
            config: self.config.clone(),
        };

        let app = Router::new()
            .route("/.well-known/solid", get(discovery_handler))
            .route("/subscription", post(subscription_handler))
            .route("/channel/ws/{id}", get(websocket_handler))
            .route("/channel/sse/{id}", get(sse_handler))
            .with_state(state);

        let addr = format!("0.0.0.0:{}", self.config.bind_port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!(addr = %addr, "CDC notification server listening");
        axum::serve(listener, app).await?;
        Ok(())
    }
}

// --- Handlers ---

async fn discovery_handler(State(state): State<AppState>) -> Json<JsonValue> {
    let base = format!("http://0.0.0.0:{}", state.config.bind_port);
    Json(json!({
        "@context": "https://www.w3.org/ns/solid/notifications/v1",
        "id": format!("{base}/.well-known/solid"),
        "subscription": format!("{base}/subscription"),
        "channelType": [
            "WebSocketChannel2023",
            "StreamingHTTPChannel2023"
        ]
    }))
}

#[derive(Deserialize)]
struct SubscriptionRequest {
    #[serde(rename = "type")]
    channel_type: String,
    topic: Option<String>,
    state: Option<String>,
    #[expect(dead_code, reason = "rate limiting not yet implemented")]
    rate: Option<String>,
}

async fn subscription_handler(
    State(state): State<AppState>,
    Json(req): Json<SubscriptionRequest>,
) -> Result<Json<JsonValue>, (axum::http::StatusCode, String)> {
    let channel_type = match req.channel_type.as_str() {
        "WebSocketChannel2023" => ChannelType::WebSocketChannel2023,
        "StreamingHTTPChannel2023" => ChannelType::StreamingHTTPChannel2023,
        other => {
            return Err((
                axum::http::StatusCode::BAD_REQUEST,
                format!("Unsupported channel type: {other}"),
            ));
        }
    };

    let sub_id = uuid::Uuid::new_v4().to_string();
    let topic = req
        .topic
        .unwrap_or_else(|| state.config.main_server_url.clone());
    let last_event_id = req.state.as_ref().and_then(|s| s.parse::<u64>().ok());

    let sub = Subscription {
        id: sub_id.clone(),
        topic: topic.clone(),
        channel_type,
        created: Instant::now(),
        last_event_id,
    };
    state.registry.insert(sub);

    let base = format!("http://0.0.0.0:{}", state.config.bind_port);
    let channel_path = match channel_type {
        ChannelType::WebSocketChannel2023 => format!("{base}/channel/ws/{sub_id}"),
        ChannelType::StreamingHTTPChannel2023 => format!("{base}/channel/sse/{sub_id}"),
    };

    Ok(Json(json!({
        "@context": "https://www.w3.org/ns/solid/notifications/v1",
        "id": channel_path,
        "type": req.channel_type,
        "topic": topic
    })))
}

async fn websocket_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<axum::response::Response, (axum::http::StatusCode, String)> {
    let _sub = state.registry.get(&id).ok_or_else(|| {
        (
            axum::http::StatusCode::NOT_FOUND,
            "Subscription not found".to_owned(),
        )
    })?;

    let rx = state.sender.subscribe();
    let config = state.config.clone();
    let registry = Arc::clone(&state.registry);
    let sub_id = id.clone();

    Ok(ws.on_upgrade(move |socket| async move {
        handle_websocket(socket, rx, config, sub_id, registry).await;
    }))
}

async fn handle_websocket(
    mut socket: ws::WebSocket,
    mut rx: broadcast::Receiver<ChangeEvent>,
    config: CdcConfig,
    sub_id: String,
    registry: Arc<SubscriptionRegistry>,
) {
    let server_url = &config.main_server_url;
    let batch_window = config.batch_window;

    loop {
        // Wait for the first event (with keepalive timeout)
        let first = tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        let msg = json!({"type": "error", "message": format!("Lagged: missed {n} events")});
                        if socket.send(ws::Message::Text(msg.to_string().into())).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            () = tokio::time::sleep(Duration::from_secs(30)) => {
                // Send keepalive ping
                if socket.send(ws::Message::Ping(vec![].into())).await.is_err() {
                    break;
                }
                continue;
            }
        };

        // Batch collection within the window
        let mut batch = vec![first];
        let deadline = Instant::now() + batch_window;
        while Instant::now() < deadline && batch.len() < 100 {
            match tokio::time::timeout(
                deadline.saturating_duration_since(Instant::now()),
                rx.recv(),
            )
            .await
            {
                Ok(Ok(event)) => batch.push(event),
                Ok(Err(_)) | Err(_) => break,
            }
        }

        // Format and send
        let msg = notification::format_batch_notification(&batch, server_url);
        if socket
            .send(ws::Message::Text(msg.to_string().into()))
            .await
            .is_err()
        {
            break;
        }
    }

    // Cleanup
    registry.remove(&sub_id);
    tracing::debug!(subscription_id = %sub_id, "WebSocket subscription closed");
}

async fn sse_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<
    Sse<impl tokio_stream::Stream<Item = Result<sse::Event, std::convert::Infallible>>>,
    (axum::http::StatusCode, String),
> {
    let _sub = state.registry.get(&id).ok_or_else(|| {
        (
            axum::http::StatusCode::NOT_FOUND,
            "Subscription not found".to_owned(),
        )
    })?;

    let rx = state.sender.subscribe();
    let config = state.config.clone();

    let stream = BroadcastStream::new(rx).filter_map(move |result| match result {
        Ok(event) => {
            let msg = notification::format_notification(&event, &config.main_server_url);
            let sse_event = sse::Event::default()
                .id(event.id.to_string())
                .event("notification")
                .json_data(msg)
                .unwrap_or_else(|_| sse::Event::default().data("error"));
            Some(Ok(sse_event))
        }
        Err(_) => None,
    });

    Ok(Sse::new(stream).keep_alive(
        sse::KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("keepalive"),
    ))
}
