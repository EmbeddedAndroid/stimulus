use crate::api::{ApiError, AppState};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use lp_project::Settings;
use serde::Serialize;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/setup", get(get_setup).put(put_setup))
        .route("/api/setup/validate", post(validate))
}

#[derive(Serialize)]
struct SetupResult {
    setup: Settings,
    warnings: Vec<String>,
    hardware_reconfigure: bool,
}

async fn get_setup(State(state): State<AppState>) -> Result<Json<Settings>, ApiError> {
    state.setup().map(Json)
}

async fn put_setup(
    State(state): State<AppState>,
    Json(setup): Json<Settings>,
) -> Result<Json<SetupResult>, ApiError> {
    let hardware_reconfigure = state.apply_setup(setup.clone())?;
    Ok(Json(SetupResult {
        setup,
        warnings: Vec::new(),
        hardware_reconfigure,
    }))
}

async fn validate(
    State(state): State<AppState>,
    Json(setup): Json<Settings>,
) -> Result<Json<SetupResult>, ApiError> {
    let current = state.setup()?;
    let hardware_reconfigure = (current.sample.rate_index < 2) != (setup.sample.rate_index < 2);
    state.validate_setup(&setup)?;
    Ok(Json(SetupResult {
        setup,
        warnings: Vec::new(),
        hardware_reconfigure,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use lp_project::Capture;
    use tower::ServiceExt;

    async fn request(
        state: AppState,
        method: &str,
        uri: &str,
        body: Vec<u8>,
    ) -> (StatusCode, Vec<u8>) {
        let response = crate::api::router(state)
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap_or_else(|error| panic!("{error}")),
            )
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let status = response.status();
        let body = to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap_or_else(|error| panic!("{error}"))
            .to_vec();
        (status, body)
    }

    #[tokio::test]
    async fn setup_round_trips_and_applies_to_simulator() {
        let state = AppState::new();
        let (status, body) = request(state.clone(), "GET", "/api/setup", Vec::new()).await;
        assert_eq!(status, StatusCode::OK);
        let mut setup: Settings =
            serde_json::from_slice(&body).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(setup.sample.rate_index, 7);
        setup.sample.rate_index = 7;
        setup.sample.rate_hz = 10_000_000;
        let body = serde_json::to_vec(&setup).unwrap_or_else(|error| panic!("{error}"));
        let (status, response) = request(state.clone(), "PUT", "/api/setup", body).await;
        assert_eq!(status, StatusCode::OK);
        let value: serde_json::Value =
            serde_json::from_slice(&response).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(value["setup"]["sample"]["rate_hz"], 10_000_000);
        assert_eq!(value["hardware_reconfigure"], false);

        let (status, capture) = request(state, "POST", "/api/acquire", b"{}".to_vec()).await;
        assert_eq!(status, StatusCode::OK);
        let capture: Capture =
            serde_json::from_slice(&capture).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(capture.sample_period_s, 1e-7);
    }

    #[tokio::test]
    async fn validation_rejects_rate_mismatch_and_compression_at_500mhz() {
        let state = AppState::new();
        let mut setup = state.setup().unwrap_or_else(|error| panic!("{:?}", error));
        setup.sample.rate_index = 1;
        setup.sample.rate_hz = 500_000_000;
        setup.sample.compression = true;
        let body = serde_json::to_vec(&setup).unwrap_or_else(|error| panic!("{error}"));
        let (status, body) = request(state, "POST", "/api/setup/validate", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let value: serde_json::Value =
            serde_json::from_slice(&body).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(value["error"]["code"], "INVALID_ARG");
    }
}
