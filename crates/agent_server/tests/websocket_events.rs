//! Client-level tests for the optional breakpoint WebSocket (#451).

use std::sync::Arc;
use std::time::Duration;

use agent_client::AgentClient;
use agent_server::routes::{router, AppState};
use control_plane::ControlPlane;
use futures_util::StreamExt;
use spec_chum_host::ModelId;
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};

async fn start_server(
    plane: Arc<ControlPlane>,
    token: Option<&str>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    let app = router(AppState {
        plane,
        token: token.map(str::to_owned),
        insecure: token.is_none(),
    });
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test router");
    });
    (format!("http://{address}"), task)
}

fn loaded_plane() -> Arc<ControlPlane> {
    let plane = Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
    let synthetic_rom = vec![0; 16 * 1024];
    plane
        .load_rom_bytes(&synthetic_rom)
        .expect("load synthetic ROM");
    plane
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_delivers_live_breakpoint_and_reconnect_snapshot_once() {
    let plane = loaded_plane();
    let (base, server) = start_server(plane.clone(), None).await;
    let client = AgentClient::new(&base, None);
    let mut events = client
        .connect_breakpoint_events()
        .await
        .expect("connect event client");

    plane.add_breakpoint(0).expect("add breakpoint");
    let result = plane
        .lock_host()
        .run_frames(1)
        .expect("live frame loop reaches breakpoint");
    assert_eq!(format!("{result:?}"), "Pc(0)");

    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("wait for live breakpoint event")
        .expect("receive live breakpoint event")
        .expect("event stream remains open");
    assert_eq!(event.version, 1);
    assert_eq!(event.sequence, 1);
    assert_eq!(event.event, "breakpoint.hit");
    assert_eq!(event.delivery, "live");
    assert_eq!(event.pc, 0);
    assert_eq!(event.reason, "pc:0000");
    assert!(event.paused);

    events.close().await.expect("close first client");

    let reconnect_client = AgentClient::new(&base, None);
    let mut reconnected = reconnect_client
        .connect_breakpoint_events()
        .await
        .expect("reconnect event client");
    let snapshot = tokio::time::timeout(Duration::from_secs(2), reconnected.recv())
        .await
        .expect("wait for reconnect snapshot")
        .expect("receive reconnect snapshot")
        .expect("event stream remains open");
    assert_eq!(snapshot.version, 1);
    assert_eq!(snapshot.sequence, 1);
    assert_eq!(snapshot.event, "breakpoint.hit");
    assert_eq!(snapshot.delivery, "snapshot");
    assert_eq!(snapshot.pc, 0);

    plane.continue_execution().expect("continue execution");
    let next = tokio::time::timeout(Duration::from_millis(160), reconnected.recv()).await;
    assert!(
        next.is_err(),
        "unchanged breakpoint state must not be repeated"
    );
    reconnected.close().await.expect("close reconnected client");
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_rejects_missing_bearer_token_before_upgrade() {
    let plane = loaded_plane();
    let (base, server) = start_server(plane, Some("test-token")).await;
    let url = format!("{}/v1/events", base.replace("http://", "ws://"));
    let error = connect_async(&url)
        .await
        .expect_err("missing token is rejected");
    let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
        panic!("expected HTTP auth response, got {error:?}");
    };
    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_closes_with_server_error_when_debugger_state_is_unavailable() {
    let plane = Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
    let (base, server) = start_server(plane, None).await;
    let url = format!("{}/v1/events", base.replace("http://", "ws://"));
    let (mut client, _) = connect_async(&url).await.expect("connect event client");
    let message = tokio::time::timeout(Duration::from_secs(2), client.next())
        .await
        .expect("wait for debugger failure close")
        .expect("close frame is received")
        .expect("read close frame");
    match message {
        Message::Close(Some(frame)) => assert_eq!(
            frame.code,
            tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Error
        ),
        other => panic!("expected WebSocket 1011 close, got {other:?}"),
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_closes_with_server_error_if_shared_machine_becomes_unavailable() {
    let plane = loaded_plane();
    let (base, server) = start_server(plane.clone(), None).await;
    let url = format!("{}/v1/events", base.replace("http://", "ws://"));
    let (mut client, _) = connect_async(&url).await.expect("connect event client");

    plane.lock_host().set_model(ModelId::Spectrum128);

    let message = tokio::time::timeout(Duration::from_secs(2), client.next())
        .await
        .expect("wait for monitor failure close")
        .expect("close frame is received")
        .expect("read close frame");
    match message {
        Message::Close(Some(frame)) => assert_eq!(
            frame.code,
            tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Error
        ),
        other => panic!("expected WebSocket 1011 close, got {other:?}"),
    }
    server.abort();
}
