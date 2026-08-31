//! HTTP integration smoke test for the agent debug API (#210).

use std::sync::Arc;

use agent_server::routes::{router, AppState};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use control_plane::ControlPlane;
use machine::Model;
use spec_chum_host::ModelId;
use tower::ServiceExt;

fn rom48() -> Option<Vec<u8>> {
    machine::resolve_rom_path(Model::Spectrum48).and_then(|path| std::fs::read(path).ok())
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_api_run_inspect_and_framebuffer_png() {
    let Some(rom) = rom48() else {
        eprintln!("skip: Spectrum 48 ROM missing");
        return;
    };
    let plane = Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
    plane.load_rom_bytes(&rom).expect("rom");
    let app = router(AppState {
        plane: plane.clone(),
        token: None,
    });

    let health = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("health");
    assert_eq!(health.status(), StatusCode::OK);
    let body = axum::body::to_bytes(health.into_body(), usize::MAX)
        .await
        .expect("health body");
    let health: serde_json::Value = serde_json::from_slice(&body).expect("health json");
    assert_eq!(health["ok"], true);
    assert_eq!(health["has_machine"], true);

    let run = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/run")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"frames":1}"#))
                .unwrap(),
        )
        .await
        .expect("run");
    assert_eq!(run.status(), StatusCode::OK);

    let inspect = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/inspect")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("inspect");
    assert_eq!(inspect.status(), StatusCode::OK);
    let body = axum::body::to_bytes(inspect.into_body(), usize::MAX)
        .await
        .expect("inspect body");
    let inspect: serde_json::Value = serde_json::from_slice(&body).expect("inspect json");
    assert!(
        inspect.get("pc").is_some(),
        "inspect json should include pc"
    );

    let fb = app
        .oneshot(
            Request::builder()
                .uri("/v1/framebuffer?border=false&format=png")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("framebuffer");
    assert_eq!(fb.status(), StatusCode::OK);
    let body = axum::body::to_bytes(fb.into_body(), usize::MAX)
        .await
        .expect("png body");
    assert!(body.starts_with(&[0x89, b'P', b'N', b'G']));
    let meta = plane.framebuffer_meta().expect("meta");
    assert_eq!(meta.width, 256);
    assert_eq!(meta.height, 192);
}
