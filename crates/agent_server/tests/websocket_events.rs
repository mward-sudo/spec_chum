//! Client-level tests for the optional breakpoint WebSocket (#451).

use std::sync::Arc;
use std::time::Duration;

use agent_client::AgentClient;
use agent_server::routes::{router, AppState};
use control_plane::ControlPlane;
use futures_util::{SinkExt, StreamExt};
use spec_chum_host::ModelId;
use tokio::net::TcpListener;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::header::{HOST, ORIGIN},
        Message,
    },
};

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
async fn websocket_delivers_two_same_pc_stops_before_the_next_poll() {
    let plane = loaded_plane();
    let (base, server) = start_server(plane.clone(), None).await;
    let url = format!("{}/v1/events", base.replace("http://", "ws://"));
    let (mut client, _) = connect_async(&url).await.expect("connect event client");
    client
        .send(Message::Ping(Vec::new().into()))
        .await
        .expect("ping subscribed connection");
    let pong = tokio::time::timeout(Duration::from_secs(2), client.next())
        .await
        .expect("wait for pong")
        .expect("pong frame")
        .expect("read pong");
    assert!(matches!(pong, Message::Pong(_)));

    // Hold the shared host lock across both stops. The monitor cannot see the
    // resumed interval or even the first stop before the second stop arrives.
    {
        let mut host = plane.lock_host();
        host.add_breakpoint(0).expect("add breakpoint");
        assert_eq!(
            format!("{:?}", host.run_frames(1).expect("first stop")),
            "Pc(0)"
        );
        host.continue_execution().expect("resume");
        host.step().expect("consume skip-once instruction");
        host.machine_mut().expect("machine").cpu_mut().regs.pc = 0;
        assert_eq!(
            format!("{:?}", host.run_frames(1).expect("second stop")),
            "Pc(0)"
        );
    }

    let mut sequences = Vec::new();
    for _ in 0..2 {
        let message = tokio::time::timeout(Duration::from_secs(2), client.next())
            .await
            .expect("wait for each hit")
            .expect("event frame")
            .expect("read event");
        let Message::Text(body) = message else {
            panic!("expected text event, got {message:?}");
        };
        let event: agent_client::BreakpointEvent =
            serde_json::from_str(&body).expect("decode event");
        assert_eq!(event.pc, 0);
        assert_eq!(event.reason, "pc:0000");
        assert_eq!(event.delivery, "live");
        sequences.push(event.sequence);
    }
    assert_eq!(sequences, vec![1, 2]);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_hit_identity_survives_session_reset_and_machine_replacement() {
    let plane = loaded_plane();
    let (base, server) = start_server(plane.clone(), None).await;
    let url = format!("{}/v1/events", base.replace("http://", "ws://"));
    let (mut socket, _) = connect_async(&url).await.expect("connect event client");
    socket
        .send(Message::Ping(Vec::new().into()))
        .await
        .expect("ping subscribed connection");
    let pong = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("wait for pong")
        .expect("pong frame")
        .expect("read pong");
    assert!(matches!(pong, Message::Pong(_)));

    for phase in 1..=3 {
        {
            let mut host = plane.lock_host();
            match phase {
                2 => host.reset().expect("reset live machine"),
                3 => {
                    host.set_model(ModelId::Spectrum48);
                    host.load_rom_bytes(&vec![0; 16 * 1024])
                        .expect("replace machine");
                }
                _ => {}
            }
            host.add_breakpoint(0).expect("arm breakpoint");
            assert_eq!(
                format!("{:?}", host.run_frames(1).expect("reach PC stop")),
                "Pc(0)"
            );
        }
        let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .expect("wait for hit")
            .expect("event frame")
            .expect("read event");
        let Message::Text(body) = message else {
            panic!("expected text event, got {message:?}");
        };
        let event: agent_client::BreakpointEvent =
            serde_json::from_str(&body).expect("decode event");
        assert_eq!(event.sequence, phase);
        assert_eq!(event.pc, 0);
        assert_eq!(event.delivery, "live");
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn tokenless_websocket_rejects_cross_origin_but_accepts_same_origin_and_native() {
    let plane = loaded_plane();
    let (base, server) = start_server(plane, None).await;
    let url = format!("{}/v1/events", base.replace("http://", "ws://"));

    let mut cross_origin = url.clone().into_client_request().expect("request");
    cross_origin
        .headers_mut()
        .insert(ORIGIN, "https://attacker.example".parse().expect("origin"));
    let error = connect_async(cross_origin)
        .await
        .expect_err("cross-origin browser upgrade must fail");
    let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
        panic!("expected HTTP response, got {error:?}");
    };
    assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);

    let port = base.rsplit(':').next().expect("port");
    let rebound_host = format!("attacker.example:{port}");
    let mut rebinding = url.clone().into_client_request().expect("request");
    rebinding
        .headers_mut()
        .insert(HOST, rebound_host.parse().expect("rebound host"));
    rebinding.headers_mut().insert(
        ORIGIN,
        format!("http://{rebound_host}")
            .parse()
            .expect("rebound origin"),
    );
    let error = connect_async(rebinding)
        .await
        .expect_err("DNS rebinding style Host must fail");
    let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
        panic!("expected HTTP response, got {error:?}");
    };
    assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);

    let mut same_origin = url.clone().into_client_request().expect("request");
    same_origin
        .headers_mut()
        .insert(ORIGIN, base.parse().expect("same origin"));
    let (mut browser, _) = connect_async(same_origin)
        .await
        .expect("same-origin browser upgrade");
    browser.close(None).await.expect("close browser socket");

    let (mut native, _) = connect_async(url)
        .await
        .expect("native client without Origin");
    native.close(None).await.expect("close native socket");
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
async fn agent_client_reports_1011_as_error_and_normal_close_as_end_of_stream() {
    let plane = Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
    let (base, server) = start_server(plane, None).await;
    let client = AgentClient::new(&base, None);
    let mut events = client
        .connect_breakpoint_events()
        .await
        .expect("connect event client");
    let error = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("wait for server error close")
        .expect_err("1011 must be an error");
    assert!(error.to_string().contains("1011"), "{error}");
    assert!(
        error.to_string().contains("debugger state unavailable"),
        "{error}"
    );
    server.abort();

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind normal-close server");
    let address = listener.local_addr().expect("normal-close address");
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept client");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("accept websocket");
        socket
            .close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal,
                reason: "done".into(),
            }))
            .await
            .expect("send normal close");
    });
    let client = AgentClient::new(&format!("http://{address}"), None);
    let mut events = client
        .connect_breakpoint_events()
        .await
        .expect("connect normal-close client");
    let result = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("wait for normal close")
        .expect("normal close is not an error");
    assert!(result.is_none());
    server.await.expect("normal-close server exits");
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
