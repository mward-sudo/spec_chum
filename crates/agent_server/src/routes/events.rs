//! Optional WebSocket push events for debugger clients.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, Weak,
};
use std::time::Duration;

use axum::{
    extract::{ws, Extension, State, WebSocketUpgrade},
    http::{header, uri::Authority, HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::{api_error, check_auth, AppState};

const BREAKPOINT_EVENT_VERSION: u16 = 1;
const WATCH_INTERVAL: Duration = Duration::from_millis(100);
const EVENT_CHANNEL_CAPACITY: usize = 32;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BreakpointEvent {
    pub version: u16,
    pub sequence: u64,
    pub event: String,
    pub delivery: String,
    pub pc: u16,
    pub reason: String,
    pub paused: bool,
}

#[derive(Debug, Default)]
struct EventState {
    sequence: u64,
    last_seen_id: u64,
    last_event_id: u64,
    last_event: Option<BreakpointEvent>,
}

#[derive(Clone, Debug)]
enum HubMessage {
    Event(BreakpointEvent),
    Unavailable,
}

/// One monitor per router, shared by every connected event client.
#[derive(Debug)]
pub(crate) struct BreakpointEventHub {
    plane: super::SharedPlane,
    sender: broadcast::Sender<HubMessage>,
    state: Mutex<EventState>,
    monitor_started: AtomicBool,
    unavailable: AtomicBool,
}

impl BreakpointEventHub {
    /// Create a per-router event hub for the shared control plane.
    pub(crate) fn new(plane: super::SharedPlane) -> Arc<Self> {
        let (sender, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Arc::new(Self {
            plane,
            sender,
            state: Mutex::new(EventState::default()),
            monitor_started: AtomicBool::new(false),
            unavailable: AtomicBool::new(false),
        })
    }

    /// Subscribe a connection and derive its initial current-state snapshot.
    fn subscribe(
        self: &Arc<Self>,
    ) -> Result<(broadcast::Receiver<HubMessage>, Option<BreakpointEvent>), ()> {
        let mut state = self.state.lock().map_err(|_| ())?;
        let first_subscriber = self.sender.receiver_count() == 0;
        let receiver = self.sender.subscribe();
        let snapshot = if first_subscriber {
            // The journal is for connected clients. Reconnects get only the
            // current paused stop, never the disconnected interval's history.
            let observation = self
                .plane
                .pc_breakpoint_observation(state.last_seen_id)
                .map_err(|_| ())?;
            state.last_seen_id = state.last_seen_id.max(observation.latest_id);
            observation.active.map(|hit| {
                if state.last_event_id != hit.id {
                    state.sequence = state.sequence.saturating_add(1);
                    state.last_event_id = hit.id;
                    state.last_event = Some(event_for(hit.pc, state.sequence, "live"));
                }
                let event = state.last_event.as_ref().expect("active hit has an event");
                BreakpointEvent {
                    delivery: "snapshot".into(),
                    ..event.clone()
                }
            })
        } else {
            self.observe_locked(&mut state)?;
            let observation = self
                .plane
                .pc_breakpoint_observation(state.last_seen_id)
                .map_err(|_| ())?;
            observation.active.and_then(|hit| {
                (state.last_event_id == hit.id).then(|| BreakpointEvent {
                    delivery: "snapshot".into(),
                    ..state.last_event.clone().expect("active hit has an event")
                })
            })
        };
        drop(state);
        self.start_monitor();
        Ok((receiver, snapshot))
    }

    /// Start one shared observer while at least one connection is subscribed.
    fn start_monitor(self: &Arc<Self>) {
        if self
            .monitor_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let weak: Weak<Self> = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut poll = tokio::time::interval(WATCH_INTERVAL);
            poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                poll.tick().await;
                let Some(hub) = weak.upgrade() else {
                    return;
                };
                if hub.sender.receiver_count() == 0 {
                    continue;
                }
                if hub.observe_and_publish().is_err() {
                    if !hub.unavailable.swap(true, Ordering::AcqRel) {
                        let _ = hub.sender.send(HubMessage::Unavailable);
                    }
                } else {
                    hub.unavailable.store(false, Ordering::Release);
                }
            }
        });
    }

    /// Read the stop journal and broadcast each unseen PC stop.
    fn observe_and_publish(&self) -> Result<(), ()> {
        let mut state = self.state.lock().map_err(|_| ())?;
        self.observe_locked(&mut state)
    }

    /// Advance the hub cursor and publish unseen stops while holding its state lock.
    fn observe_locked(&self, state: &mut EventState) -> Result<(), ()> {
        let observation = self
            .plane
            .pc_breakpoint_observation(state.last_seen_id)
            .map_err(|_| ())?;
        for hit in observation.recent {
            if hit.id <= state.last_seen_id {
                continue;
            }
            state.last_seen_id = hit.id;
            state.sequence = state.sequence.saturating_add(1);
            let event = event_for(hit.pc, state.sequence, "live");
            state.last_event_id = hit.id;
            state.last_event = Some(event.clone());
            let _ = self.sender.send(HubMessage::Event(event));
        }
        state.last_seen_id = state.last_seen_id.max(observation.latest_id);
        Ok(())
    }
}

/// Build the wire payload for one PC breakpoint stop.
fn event_for(pc: u16, sequence: u64, delivery: &str) -> BreakpointEvent {
    BreakpointEvent {
        version: BREAKPOINT_EVENT_VERSION,
        sequence,
        event: "breakpoint.hit".into(),
        delivery: delivery.into(),
        pc,
        reason: format!("pc:{pc:04X}"),
        paused: true,
    }
}

/// Authenticate and upgrade a request to the breakpoint event stream.
pub(crate) async fn websocket(
    State(state): State<AppState>,
    Extension(events): Extension<Arc<BreakpointEventHub>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if let Err(error) = check_auth(&state, &headers) {
        return api_error(&state.plane, error);
    }
    if state.token.is_none() && !same_origin_or_native(&headers) {
        return (StatusCode::FORBIDDEN, "cross-origin WebSocket denied").into_response();
    }

    upgrade
        .on_upgrade(move |socket| watch_breakpoints(socket, events))
        .into_response()
}

/// Send the initial snapshot, then stream live stops until close or failure.
async fn watch_breakpoints(mut socket: ws::WebSocket, events: Arc<BreakpointEventHub>) {
    let Ok((mut receiver, snapshot)) = events.subscribe() else {
        close_with_error(&mut socket, "debugger state unavailable").await;
        return;
    };
    let mut last_sequence = 0;
    if let Some(snapshot) = snapshot {
        last_sequence = snapshot.sequence;
        if send_event(&mut socket, &snapshot).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            event = receiver.recv() => {
                match event {
                    Ok(HubMessage::Event(event)) if event.sequence > last_sequence => {
                        last_sequence = event.sequence;
                        if send_event(&mut socket, &event).await.is_err() {
                            return;
                        }
                    }
                    Ok(HubMessage::Unavailable) => {
                        close_with_error(&mut socket, "debugger state unavailable").await;
                        return;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        close_with_error(&mut socket, "event stream lagged; reconnect for snapshot").await;
                        return;
                    }
                    Ok(HubMessage::Event(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
            message = socket.recv() => {
                match message {
                    Some(Ok(ws::Message::Close(_)) | Err(_)) | None => return,
                    Some(Ok(ws::Message::Ping(payload))) => {
                        if socket.send(ws::Message::Pong(payload)).await.is_err() {
                            return;
                        }
                    }
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

/// Allow native clients without Origin and same-origin browser clients on loopback.
fn same_origin_or_native(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let (Ok(origin), Ok(host)) = (
        origin.to_str().ok().unwrap_or("").parse::<Uri>(),
        host.parse::<Authority>(),
    ) else {
        return false;
    };
    let Some(authority) = origin.authority() else {
        return false;
    };
    origin.scheme_str() == Some("http")
        && origin.path() == "/"
        && origin.query().is_none()
        && (host.host().eq_ignore_ascii_case("localhost")
            || matches!(host.host(), "127.0.0.1" | "[::1]" | "::1"))
        && authority.host().eq_ignore_ascii_case(host.host())
        && authority.port_u16().unwrap_or(80) == host.port_u16().unwrap_or(80)
}

/// Serialize and send one event as a WebSocket text frame.
async fn send_event(socket: &mut ws::WebSocket, event: &BreakpointEvent) -> Result<(), ()> {
    let body = serde_json::to_string(event).map_err(|_| ())?;
    socket
        .send(ws::Message::Text(body.into()))
        .await
        .map_err(|_| ())
}

/// Close the stream with a server-error code and a reconnectable reason.
async fn close_with_error(socket: &mut ws::WebSocket, reason: &'static str) {
    let _ = socket
        .send(ws::Message::Close(Some(ws::CloseFrame {
            code: ws::close_code::ERROR,
            reason: reason.into(),
        })))
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use control_plane::ControlPlane;
    use spec_chum_host::ModelId;

    #[tokio::test]
    async fn monitor_skips_observation_when_no_subscribers_remain() {
        let plane = Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
        plane
            .load_rom_bytes(&vec![0; 16 * 1024])
            .expect("load synthetic ROM");
        let hub = BreakpointEventHub::new(plane.clone());
        let (receiver, _) = hub.subscribe().expect("subscribe");
        assert!(hub.monitor_started.load(Ordering::Acquire));
        drop(receiver);
        plane.lock_host().set_model(ModelId::Spectrum128);
        tokio::time::sleep(WATCH_INTERVAL + Duration::from_millis(20)).await;
        assert!(!hub.unavailable.load(Ordering::Acquire));
        assert!(hub.monitor_started.load(Ordering::Acquire));
    }
}
