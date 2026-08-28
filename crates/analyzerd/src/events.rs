use crate::api::{ApiError, AppState};
use axum::{
    Json, Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub r#type: String,
    pub seq: u64,
    pub ts: u64,
    #[serde(flatten)]
    pub data: Value,
}
impl Event {
    pub fn new(seq: u64, kind: &str, data: Value) -> Self {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| {
                duration.as_millis().min(u128::from(u64::MAX)) as u64
            });
        Self {
            r#type: kind.to_owned(),
            seq,
            ts,
            data,
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/events", get(events))
        .route("/ws", get(ws))
}

#[derive(Deserialize)]
struct EventsQuery {
    since_seq: Option<u64>,
    limit: Option<usize>,
}
async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<Vec<Event>>, ApiError> {
    state
        .events_since(
            query.since_seq.unwrap_or(0),
            query.limit.unwrap_or(100).min(1000),
        )
        .map(Json)
}

async fn ws(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| stream(socket, state))
}

async fn stream(mut socket: WebSocket, state: AppState) {
    let mut receiver = state.subscribe();
    loop {
        tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            },
            event = receiver.recv() => match event {
                Ok(event) => {
                    let Ok(text) = serde_json::to_string(&event) else { continue };
                    if socket.send(Message::Text(text.into())).await.is_err() { break; }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_filters_sequences_and_caps_limit() {
        let state = AppState::new();
        state.emit("status", serde_json::json!({"state":"idle"}));
        state.emit("log", serde_json::json!({"message":"ready"}));
        let events = state
            .events_since(1, 100)
            .unwrap_or_else(|error| panic!("{:?}", error));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].r#type, "log");
        assert_eq!(events[0].seq, 2);
    }
}
