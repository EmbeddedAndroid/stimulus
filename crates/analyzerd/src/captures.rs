use crate::api::{ApiError, AppState, api_error};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{Response, header},
    routing::get,
};
use lp_core::api::binary::{
    self, FLAG_CHANNEL_SUBSET, FLAG_LAST, Header, Kind, RleSlot, SLOT_CLK1, SLOT_CLK2,
    SLOT_CONTINUES, SLOT_REFERENCE, SLOT_TRIGGER,
};
use lp_project::{Capture, CaptureSummary, summarize};
use serde::Deserialize;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/captures", get(list))
        .route("/api/captures/{id}", get(get_capture))
        .route("/api/captures/{id}/summary", get(summary))
        .route("/api/captures/{id}/data", get(data))
}

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<usize>,
}
async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<Capture>>, ApiError> {
    let limit = query.limit.unwrap_or(50).min(200);
    state
        .captures()
        .list(limit)
        .map(Json)
        .map_err(|e| api_error("INTERNAL", e.to_string()))
}
async fn get_capture(
    State(state): State<AppState>,
    Path(id): Path<u32>,
) -> Result<Json<Capture>, ApiError> {
    capture(&state, id).map(Json)
}
async fn summary(
    State(state): State<AppState>,
    Path(id): Path<u32>,
) -> Result<Json<CaptureSummary>, ApiError> {
    let capture = capture(&state, id)?;
    summarize(&capture)
        .map(Json)
        .map_err(|e| api_error("INTERNAL", e.to_string()))
}

#[derive(Deserialize)]
struct DataQuery {
    format: Option<String>,
    channels: Option<String>,
    from: Option<u64>,
    to: Option<u64>,
}
async fn data(
    State(state): State<AppState>,
    Path(id): Path<u32>,
    Query(query): Query<DataQuery>,
) -> Result<Response<Body>, ApiError> {
    let capture = capture(&state, id)?;
    let from = query.from.unwrap_or(0);
    let to = query.to.unwrap_or_else(|| capture.expanded_len());
    if from >= to || to > capture.expanded_len() {
        return Err(api_error(
            "INVALID_ARG",
            "capture range is outside available samples",
        ));
    }
    let mask = channel_mask(query.channels.as_deref())?;
    let bytes = match query.format.as_deref().unwrap_or("rle") {
        "rle" => rle_bytes(&capture, from, to, mask)?,
        "expanded" => expanded_bytes(&capture, from, to, mask)?,
        other => {
            return Err(api_error(
                "INVALID_ARG",
                format!("unknown capture format: {other}"),
            ));
        }
    };
    Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes))
        .map_err(|e| api_error("INTERNAL", e.to_string()))
}
fn capture(state: &AppState, id: u32) -> Result<Capture, ApiError> {
    state
        .captures()
        .get(id)
        .map_err(|e| api_error("INTERNAL", e.to_string()))?
        .ok_or_else(|| api_error("UNKNOWN_CAPTURE", format!("unknown capture: {id}")))
}
fn channel_mask(channels: Option<&str>) -> Result<u64, ApiError> {
    let Some(channels) = channels else {
        return Ok((1_u64 << 34) - 1);
    };
    let mut mask = 0_u64;
    for name in channels.split(',') {
        let name = name.trim();
        let channel = if name.eq_ignore_ascii_case("CLK1") {
            32
        } else if name.eq_ignore_ascii_case("CLK2") {
            33
        } else {
            name.strip_prefix('D')
                .or_else(|| name.strip_prefix('d'))
                .ok_or_else(|| api_error("INVALID_ARG", format!("unknown channel: {name}")))?
                .parse::<u8>()
                .map_err(|_| api_error("INVALID_ARG", format!("unknown channel: {name}")))?
        };
        if channel >= 34 {
            return Err(api_error("INVALID_ARG", format!("unknown channel: {name}")));
        }
        mask |= 1_u64 << channel;
    }
    if mask == 0 {
        return Err(api_error("INVALID_ARG", "channel subset is empty"));
    }
    Ok(mask)
}
fn base_header(capture: &Capture, kind: Kind, mask: u64) -> Header {
    Header {
        kind,
        flags: FLAG_LAST
            | if mask.count_ones() < 34 {
                FLAG_CHANNEL_SUBSET
            } else {
                0
            },
        channel_count: mask.count_ones() as u16,
        capture_id: capture.id,
        chunk_index: 0,
        chunk_count: 1,
    }
}
fn rle_bytes(capture: &Capture, from: u64, to: u64, mask: u64) -> Result<Vec<u8>, ApiError> {
    let mut slots = Vec::new();
    let mut start = 0_u64;
    for run in &capture.runs {
        let end = start + run.count;
        let piece_start = start.max(from);
        let piece_end = end.min(to);
        if piece_start < piece_end {
            let data = run.data & mask;
            let mut flags = 0;
            if data & (1_u64 << 32) != 0 {
                flags |= SLOT_CLK1;
            }
            if data & (1_u64 << 33) != 0 {
                flags |= SLOT_CLK2;
            }
            if capture.trigger_sample >= piece_start && capture.trigger_sample < piece_end {
                flags |= SLOT_TRIGGER;
            }
            if capture.reference_sample >= piece_start && capture.reference_sample < piece_end {
                flags |= SLOT_REFERENCE;
            }
            if piece_start > start || piece_end < end {
                flags |= SLOT_CONTINUES;
            }
            let mut count = piece_end - piece_start;
            while count > 0 {
                let take = count.min(u64::from(u32::MAX));
                slots.push(RleSlot {
                    data: data as u32,
                    flags,
                    count: take as u32,
                });
                count -= take;
            }
        }
        start = end;
        if start >= to {
            break;
        }
    }
    binary::encode_rle(base_header(capture, Kind::Rle, mask), &slots)
        .map_err(|e| api_error("INTERNAL", e.to_string()))
}
fn expanded_bytes(capture: &Capture, from: u64, to: u64, mask: u64) -> Result<Vec<u8>, ApiError> {
    let length = to - from;
    if length > 1_048_576 {
        return Err(api_error(
            "RANGE_TOO_LARGE",
            "expanded capture range exceeds 1,048,576 samples",
        ));
    }
    let mut samples = Vec::with_capacity(length as usize);
    for sample in from..to {
        let mut value = capture
            .sample_at(sample)
            .ok_or_else(|| api_error("INTERNAL", "capture sample index failed"))?
            & mask;
        if sample == capture.trigger_sample {
            value |= 1_u64 << 40;
        }
        if sample == capture.reference_sample {
            value |= 1_u64 << 41;
        }
        samples.push(value);
    }
    binary::encode_expanded(base_header(capture, Kind::Expanded, mask), &samples)
        .map_err(|e| api_error("INTERNAL", e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use lp_core::api::binary::{decode_expanded, decode_rle};
    use lp_project::Run;
    use tower::ServiceExt;

    fn state_with_capture() -> AppState {
        let state = AppState::new();
        let mut capture = Capture::new(
            0,
            1e-6,
            3,
            vec![
                Run { data: 0, count: 2 },
                Run {
                    data: 1 | (1 << 32),
                    count: 3,
                },
                Run {
                    data: 2 | (1 << 33),
                    count: 2,
                },
            ],
        )
        .unwrap_or_else(|error| panic!("{error}"));
        capture.reference_sample = 5;
        state
            .insert_capture(capture)
            .unwrap_or_else(|error| panic!("{:?}", error));
        state
    }

    async fn get(state: AppState, uri: &str) -> (StatusCode, String, Vec<u8>) {
        let response = crate::api::router(state)
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("{error}")),
            )
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap_or_else(|error| panic!("{error}"))
            .to_vec();
        (status, content_type, body)
    }

    #[tokio::test]
    async fn lists_gets_and_summarizes_captures() {
        let state = state_with_capture();
        let (status, _, body) = get(state.clone(), "/api/captures?limit=1").await;
        assert_eq!(status, StatusCode::OK);
        let listed: Vec<Capture> =
            serde_json::from_slice(&body).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, 1);

        let (status, _, body) = get(state.clone(), "/api/captures/1").await;
        assert_eq!(status, StatusCode::OK);
        let capture: Capture =
            serde_json::from_slice(&body).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(capture.expanded_len(), 7);

        let (status, _, body) = get(state, "/api/captures/1/summary").await;
        assert_eq!(status, StatusCode::OK);
        let summary: CaptureSummary =
            serde_json::from_slice(&body).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(summary.expanded_len, 7);
        assert_eq!(summary.channels[0].transitions, 2);
    }

    #[tokio::test]
    async fn ranged_rle_preserves_markers_and_subset() {
        let (status, content_type, body) = get(
            state_with_capture(),
            "/api/captures/1/data?format=rle&channels=D0,%20CLK1&from=1&to=6",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, "application/octet-stream");
        let (header, slots) = decode_rle(&body).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(header.capture_id, 1);
        assert_eq!(header.channel_count, 2);
        assert_eq!(header.flags, FLAG_LAST | FLAG_CHANNEL_SUBSET);
        assert_eq!(
            slots.iter().map(|slot| u64::from(slot.count)).sum::<u64>(),
            5
        );
        assert!(slots[0].flags & SLOT_CONTINUES != 0);
        assert!(slots[1].flags & SLOT_TRIGGER != 0);
        assert!(slots[1].flags & SLOT_CLK1 != 0);
        assert!(slots[2].flags & SLOT_REFERENCE != 0);
        assert_eq!(slots[2].data, 0);
    }

    #[tokio::test]
    async fn expanded_marks_trigger_reference_and_rejects_bad_requests() {
        let state = state_with_capture();
        let (status, _, body) = get(
            state.clone(),
            "/api/captures/1/data?format=expanded&from=2&to=6",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (header, samples) = decode_expanded(&body).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(header.kind, Kind::Expanded);
        assert_eq!(samples.len(), 4);
        assert!(samples[1] & (1 << 40) != 0);
        assert!(samples[3] & (1 << 41) != 0);

        for (uri, expected) in [
            ("/api/captures/99", StatusCode::NOT_FOUND),
            ("/api/captures/1/data?channels=D34", StatusCode::BAD_REQUEST),
            ("/api/captures/1/data?from=6&to=2", StatusCode::BAD_REQUEST),
        ] {
            let (status, _, body) = get(state.clone(), uri).await;
            assert_eq!(status, expected);
            let envelope: serde_json::Value =
                serde_json::from_slice(&body).unwrap_or_else(|error| panic!("{error}"));
            assert!(envelope.get("error").is_some());
        }
    }
}
