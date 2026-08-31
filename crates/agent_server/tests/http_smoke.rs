//! HTTP integration smoke test for the agent debug API (#210).

use std::path::PathBuf;
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

#[tokio::test(flavor = "multi_thread")]
async fn agent_api_media_insert_requires_machine() {
    let plane = Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
    let app = router(AppState {
        plane: plane.clone(),
        token: None,
        insecure: true,
    });

    for route in ["/v1/rzx", "/v1/dsk", "/v1/trd"] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(route)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path":"/tmp/missing.media"}"#))
                    .unwrap(),
            )
            .await
            .expect(route);
        assert_eq!(
            resp.status(),
            StatusCode::CONFLICT,
            "{route} without machine should return 409"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_api_dsk_rejects_non_plus3() {
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

    // Minimal parseable MV-CPC DSK (one empty track header).
    let mut dsk = vec![0u8; 0x100];
    dsk[0..8].copy_from_slice(b"MV - CPC");
    dsk[0x30] = 1;
    dsk[0x31] = 1;
    let track_size: u16 = 0x100;
    dsk[0x32..0x34].copy_from_slice(&track_size.to_le_bytes());
    let mut track = vec![0u8; track_size as usize];
    track[0..12].copy_from_slice(b"Track-Info\r\n");
    dsk.extend_from_slice(&track);

    let dir = std::env::temp_dir().join("spec_chum_agent_api_dsk_reject");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("reject.dsk");
    std::fs::write(&path, &dsk).expect("write dsk");

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/dsk")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({ "path": path })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .expect("load dsk");
    assert!(
        resp.status().is_server_error(),
        "48K should reject DSK insert"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("error body");
    let err: serde_json::Value = serde_json::from_slice(&body).expect("error json");
    let msg = err["error"].as_str().unwrap_or("");
    assert!(
        msg.contains("+3") || msg.contains("Plus3") || msg.contains("plus3"),
        "expected model-rejection message, got {msg}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_api_joystick_kempston_mask() {
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

    let set = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/joystick")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"mask":17}"#))
                .unwrap(),
        )
        .await
        .expect("set joystick");
    assert_eq!(set.status(), StatusCode::NO_CONTENT);

    let clear = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/joystick")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"clear":true}"#))
                .unwrap(),
        )
        .await
        .expect("clear joystick");
    assert_eq!(clear.status(), StatusCode::NO_CONTENT);

    let bad = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/joystick")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .expect("bad joystick body");
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

    let no_machine = router(AppState {
        plane: Arc::new(ControlPlane::new(ModelId::Spectrum48, false)),
        token: None,
        insecure: true,
    })
    .oneshot(
        Request::builder()
            .method("POST")
            .uri("/v1/joystick")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"mask":1}"#))
            .unwrap(),
    )
    .await
    .expect("no machine");
    assert_eq!(no_machine.status(), StatusCode::CONFLICT);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_api_prefs_mouse_eject_and_continue() {
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

    let prefs = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/prefs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("get prefs");
    assert_eq!(prefs.status(), StatusCode::OK);
    let body = axum::body::to_bytes(prefs.into_body(), usize::MAX)
        .await
        .expect("prefs body");
    let prefs: serde_json::Value = serde_json::from_slice(&body).expect("prefs json");
    assert_eq!(prefs["throttle"], true);
    assert_eq!(prefs["muted"], false);

    let patched = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/v1/prefs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"muted":true,"volume":0.25,"joystick_mode":"cursor","kempston_mouse":true}"#,
                ))
                .unwrap(),
        )
        .await
        .expect("patch prefs");
    assert_eq!(patched.status(), StatusCode::OK);
    let body = axum::body::to_bytes(patched.into_body(), usize::MAX)
        .await
        .expect("patched body");
    let prefs: serde_json::Value = serde_json::from_slice(&body).expect("patched json");
    assert_eq!(prefs["muted"], true);
    assert_eq!(prefs["volume"], 0.25);
    assert_eq!(prefs["joystick_mode"], "cursor");
    assert_eq!(prefs["kempston_mouse"], true);

    let mouse = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/mouse")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"dx":5,"dy":-2,"left":true}"#))
                .unwrap(),
        )
        .await
        .expect("mouse");
    assert_eq!(mouse.status(), StatusCode::NO_CONTENT);

    let clear_mouse = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/mouse")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"clear":true}"#))
                .unwrap(),
        )
        .await
        .expect("clear mouse");
    assert_eq!(clear_mouse.status(), StatusCode::NO_CONTENT);

    let tap = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/tape/minimal_code.tap");
    assert!(tap.is_file(), "fixture missing: {}", tap.display());
    let open = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tape/open")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({ "path": tap })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .expect("tape open");
    assert_eq!(open.status(), StatusCode::NO_CONTENT);
    assert!(plane.status().expect("status").has_tape);

    let eject = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tape/eject")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("tape eject");
    assert_eq!(eject.status(), StatusCode::NO_CONTENT);
    assert!(!plane.status().expect("status").has_tape);

    plane.add_breakpoint(0x0000).expect("bp");
    let hit = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/run-until")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"max_insns":100000}"#))
                .unwrap(),
        )
        .await
        .expect("run-until");
    assert_eq!(hit.status(), StatusCode::OK);
    let body = axum::body::to_bytes(hit.into_body(), usize::MAX)
        .await
        .expect("run-until body");
    let until: serde_json::Value = serde_json::from_slice(&body).expect("run-until json");
    assert_eq!(until["paused"], true);

    let cont = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/continue")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("continue");
    assert_eq!(cont.status(), StatusCode::OK);
    let body = axum::body::to_bytes(cont.into_body(), usize::MAX)
        .await
        .expect("continue body");
    let cont: serde_json::Value = serde_json::from_slice(&body).expect("continue json");
    assert_eq!(cont["paused"], false);
    assert_eq!(cont["reason"], "none");
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_api_hardware_attach_multiface_and_divmmc() {
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

    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/hardware")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("hardware status");
    assert_eq!(status.status(), StatusCode::OK);
    let body = axum::body::to_bytes(status.into_body(), usize::MAX)
        .await
        .expect("hw body");
    let hw: serde_json::Value = serde_json::from_slice(&body).expect("hw json");
    assert_eq!(hw["has_multiface"], false);
    assert_eq!(hw["has_divmmc"], false);
    assert_eq!(hw["has_interface1"], false);

    let dir = std::env::temp_dir().join("spec_chum_agent_api_hw");
    std::fs::create_dir_all(&dir).expect("create hw fixture dir");
    let mf = dir.join("mf1.rom");
    std::fs::write(&mf, vec![0u8; 8 * 1024]).expect("mf rom");

    let attach = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/hardware/multiface")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({ "path": mf })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .expect("attach mf");
    assert_eq!(attach.status(), StatusCode::NO_CONTENT);

    let nmi = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/hardware/multiface/nmi")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("nmi");
    assert_eq!(nmi.status(), StatusCode::NO_CONTENT);

    let div = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/hardware/divmmc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("divmmc");
    assert_eq!(div.status(), StatusCode::NO_CONTENT);

    let if1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/hardware/interface1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("if1");
    assert_eq!(if1.status(), StatusCode::NO_CONTENT);

    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/hardware")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("hardware status after");
    let body = axum::body::to_bytes(status.into_body(), usize::MAX)
        .await
        .expect("hw body");
    let hw: serde_json::Value = serde_json::from_slice(&body).expect("hw json");
    assert_eq!(hw["has_multiface"], true);
    assert_eq!(hw["has_divmmc"], true);
    assert_eq!(hw["has_interface1"], true);

    // Multiface is 48K-only — reject on 128K.
    let plane128 = Arc::new(ControlPlane::new(ModelId::Spectrum128, false));
    if let Some(rom128) =
        machine::resolve_rom_path(Model::Spectrum128).and_then(|p| std::fs::read(p).ok())
    {
        plane128.load_rom_bytes(&rom128).expect("128 rom");
        let bad = router(AppState {
            plane: plane128,
            token: None,
            insecure: true,
        })
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/hardware/multiface")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({ "path": mf })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .expect("mf on 128");
        assert!(
            bad.status().is_server_error() || bad.status().is_client_error(),
            "128K must reject Multiface 1"
        );
    }

    let no_machine = router(AppState {
        plane: Arc::new(ControlPlane::new(ModelId::Spectrum48, false)),
        token: None,
        insecure: true,
    })
    .oneshot(
        Request::builder()
            .method("POST")
            .uri("/v1/hardware/divmmc")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .expect("no machine");
    assert_eq!(no_machine.status(), StatusCode::CONFLICT);
}
