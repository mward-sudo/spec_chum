//! HTTP integration smoke test for the agent debug API (#210).

use std::sync::Arc;
use std::time::Duration;

use agent_server::routes::{router, AppState};
use control_plane::ControlPlane;
use spec_chum_host::ModelId;

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rom48() -> Option<Vec<u8>> {
    std::fs::read(workspace_root().join("roms/48.rom")).ok()
}

#[tokio::test]
async fn agent_api_run_inspect_and_framebuffer_png() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/48.rom missing");
        return;
    };
    let plane = Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
    plane.load_rom_bytes(&rom).expect("rom");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let state = AppState {
        plane: plane.clone(),
        token: None,
    };
    let app = router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let base = format!("http://{addr}");
    let client = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .new_agent();
    let health: serde_json::Value = client
        .get(format!("{base}/v1/health"))
        .call()
        .expect("health")
        .into_body()
        .read_json()
        .expect("health json");
    assert_eq!(health["ok"], true);
    assert_eq!(health["has_machine"], true);
    client
        .post(format!("{base}/v1/run"))
        .send_json(serde_json::json!({ "frames": 1 }))
        .expect("run");
    let inspect: serde_json::Value = client
        .get(format!("{base}/v1/inspect"))
        .call()
        .expect("inspect")
        .into_body()
        .read_json()
        .expect("inspect json");
    assert!(inspect.get("regs").is_some());
    let png = client
        .get(format!("{base}/v1/framebuffer?border=false&format=png"))
        .call()
        .expect("framebuffer")
        .into_body()
        .read_to_vec()
        .expect("png bytes");
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    let width = client
        .get(format!("{base}/v1/framebuffer?border=false&format=png"))
        .call()
        .expect("headers")
        .headers()
        .get("x-specchum-width")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok());
    assert_eq!(width, Some(256));
}
