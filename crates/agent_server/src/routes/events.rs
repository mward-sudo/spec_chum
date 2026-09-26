//! Optional WebSocket push events for debugger clients.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, Weak,
};
use std::time::Duration;

use axum::{
    extract::{ws, Extension, State, WebSocketUpgrade},
    http::HeaderMap,
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
    previous_reason: Option<String>,
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

    fn subscribe(self: &Arc<Self>) -> broadcast::Receiver<HubMessage> {
        let receiver = self.sender.subscribe();
        self.start_monitor();
        receiver
    }

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

    fn observe_and_publish(&self) -> Result<Option<BreakpointEvent>, ()> {
        let current = self.plane.last_break().map_err(|_| ())?;
        let mut state = self.state.lock().map_err(|_| ())?;
        if !current.paused {
            state.previous_reason = None;
            return Ok(None);
        }
        let Some(pc) = breakpoint_pc(&current.reason) else {
            state.previous_reason = None;
            return Ok(None);
        };
        if state.previous_reason.as_deref() == Some(&current.reason) {
            return Ok(None);
        }

        state.sequence = state.sequence.saturating_add(1);
        let event = BreakpointEvent {
            version: BREAKPOINT_EVENT_VERSION,
            sequence: state.sequence,
            event: "breakpoint.hit".into(),
            delivery: "live".into(),
            pc,
            reason: current.reason.clone(),
            paused: true,
        };
        state.previous_reason = Some(current.reason);
        state.last_event = Some(event.clone());
        let _ = self.sender.send(HubMessage::Event(event.clone()));
        Ok(Some(event))
    }

    fn current_snapshot(&self) -> Result<Option<BreakpointEvent>, ()> {
        if let Some(event) = self.observe_and_publish()? {
            return Ok(Some(BreakpointEvent {
                delivery: "snapshot".into(),
                ..event
            }));
        }
        let state = self.state.lock().map_err(|_| ())?;
        Ok(state
            .previous_reason
            .as_ref()
            .and(state.last_event.as_ref())
            .map(|event| BreakpointEvent {
                delivery: "snapshot".into(),
                ..event.clone()
            }))
    }
}

pub(crate) async fn websocket(
    State(state): State<AppState>,
    Extension(events): Extension<Arc<BreakpointEventHub>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if let Err(error) = check_auth(&state, &headers) {
        return api_error(&state.plane, error);
    }

    upgrade
        .on_upgrade(move |socket| watch_breakpoints(socket, events))
        .into_response()
}

async fn watch_breakpoints(mut socket: ws::WebSocket, events: Arc<BreakpointEventHub>) {
    let mut receiver = events.subscribe();
    let Ok(snapshot) = events.current_snapshot() else {
        close_with_error(&mut socket).await;
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
                        close_with_error(&mut socket).await;
                        return;
                    }
                    Ok(HubMessage::Event(_)) | Err(broadcast::error::RecvError::Lagged(_)) => {}
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

fn breakpoint_pc(reason: &str) -> Option<u16> {
    u16::from_str_radix(reason.strip_prefix("pc:")?, 16).ok()
}

async fn send_event(socket: &mut ws::WebSocket, event: &BreakpointEvent) -> Result<(), ()> {
    let body = serde_json::to_string(event).map_err(|_| ())?;
    socket
        .send(ws::Message::Text(body.into()))
        .await
        .map_err(|_| ())
}

async fn close_with_error(socket: &mut ws::WebSocket) {
    let _ = socket
        .send(ws::Message::Close(Some(ws::CloseFrame {
            code: ws::close_code::ERROR,
            reason: "debugger state unavailable".into(),
        })))
        .await;
}
