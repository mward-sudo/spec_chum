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
        insecure: true,
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

#[tokio::test(flavor = "multi_thread")]
async fn agent_api_video_metadata_and_last_error() {
    let Some(rom) = rom48() else {
        eprintln!("skip: Spectrum 48 ROM missing");
        return;
    };
    let plane = Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
    plane.load_rom_bytes(&rom).expect("rom");
    let app = router(AppState {
        plane: plane.clone(),
        token: None,
        insecure: true,
    });

    let video = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/video")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("video");
    assert_eq!(video.status(), StatusCode::OK);
    let body = axum::body::to_bytes(video.into_body(), usize::MAX)
        .await
        .expect("video body");
    let video: serde_json::Value = serde_json::from_slice(&body).expect("video json");
    assert_eq!(video["width"], 256);
    assert_eq!(video["height"], 192);
    assert_eq!(video["border"], false);
    assert_eq!(video["hires"], false);

    let bad = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/keys")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"row":99,"bit":0}"#))
                .unwrap(),
        )
        .await
        .expect("bad key");
    assert!(bad.status().is_client_error() || bad.status().is_server_error());

    let err = app
        .oneshot(
            Request::builder()
                .uri("/v1/errors/last")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("last error");
    assert_eq!(err.status(), StatusCode::OK);
    let body = axum::body::to_bytes(err.into_body(), usize::MAX)
        .await
        .expect("error body");
    let last: serde_json::Value = serde_json::from_slice(&body).expect("last error json");
    assert!(
        last.get("last").and_then(|v| v.as_object()).is_some(),
        "expected last error object"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_api_keys_and_last_break() {
    let Some(rom) = rom48() else {
        eprintln!("skip: Spectrum 48 ROM missing");
        return;
    };
    let plane = Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
    plane.load_rom_bytes(&rom).expect("rom");
    let app = router(AppState {
        plane: plane.clone(),
        token: None,
        insecure: true,
    });

    let key = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/keys")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"row":6,"bit":3,"pressed":true}"#))
                .unwrap(),
        )
        .await
        .expect("set key");
    assert_eq!(key.status(), StatusCode::NO_CONTENT);

    let brk = app
        .oneshot(
            Request::builder()
                .uri("/v1/debug/last-break")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("last break");
    assert_eq!(brk.status(), StatusCode::OK);
    let body = axum::body::to_bytes(brk.into_body(), usize::MAX)
        .await
        .expect("break body");
    let brk: serde_json::Value = serde_json::from_slice(&body).expect("break json");
    assert_eq!(brk["reason"], "none");
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_api_mem_watch_list_and_add() {
    let Some(rom) = rom48() else {
        eprintln!("skip: Spectrum 48 ROM missing");
        return;
    };
    let plane = Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
    plane.load_rom_bytes(&rom).expect("rom");
    let app = router(AppState {
        plane: plane.clone(),
        token: None,
        insecure: true,
    });

    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/debug/watches")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("list watches");
    assert_eq!(list.status(), StatusCode::OK);

    let add = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/debug/watches")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"addr":"4000","write":true}"#))
                .unwrap(),
        )
        .await
        .expect("add watch");
    assert_eq!(add.status(), StatusCode::NO_CONTENT);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_api_load_rom_by_path() {
    let rom_path = match machine::resolve_rom_path(Model::Spectrum48) {
        Some(p) => p,
        None => {
            eprintln!("skip: Spectrum 48 ROM missing");
            return;
        }
    };
    let plane = Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
    let app = router(AppState {
        plane: plane.clone(),
        token: None,
        insecure: true,
    });

    let load = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/rom")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({ "path": rom_path })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .expect("load rom");
    assert_eq!(load.status(), StatusCode::NO_CONTENT);
    assert!(plane.health().expect("health").has_machine);
}
