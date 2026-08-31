use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use machine::{BreakReason, TapeLoadOptions, Watch};
use serde::Serialize;
use spec_chum_host::{
    machine_config::UserMachineConfig,
    rom_setup::{model_rom_paths_snapshot, rom_setup_json},
    HostSession, ModelId,
};
use trace::{Category, DumpFilter};

use crate::error::{ApiError, ApiResult};
use crate::framebuffer::{encode_framebuffer_png, model_slug, parse_model_slug, FramebufferMeta};

/// Loopback HTTP server configuration.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub token: Option<String>,
    /// Allow unauthenticated mutations when no token is configured (dev only).
    pub insecure: bool,
}

impl ServerConfig {
    #[must_use]
    pub fn from_env() -> Self {
        let port = std::env::var("SPEC_CHUM_AGENT_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(17_384);
        let token = std::env::var("SPEC_CHUM_AGENT_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        let insecure = std::env::var("SPEC_CHUM_AGENT_INSECURE")
            .ok()
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        Self {
            host: "127.0.0.1".into(),
            port,
            token,
            insecure,
        }
    }

    #[must_use]
    pub fn socket_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn validate_bind_host(host: &str) -> ApiResult<()> {
        match host {
            "127.0.0.1" | "::1" | "localhost" => Ok(()),
            other => Err(ApiError::BadRequest(format!(
                "refusing non-loopback bind address: {other}"
            ))),
        }
    }

    /// Refuse startup without a token unless explicit insecure opt-in (#210).
    pub fn validate_auth_config(&self) -> ApiResult<()> {
        if self.token.is_none() && !self.insecure {
            Err(ApiError::Message(
                "refusing to start agent server without SPEC_CHUM_AGENT_TOKEN;                  set SPEC_CHUM_AGENT_INSECURE=1 or pass --insecure only on trusted dev machines"
                    .into(),
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TraceFormat {
    #[default]
    Text,
    Json,
    Ndjson,
}

#[derive(Clone, Debug, Serialize)]
pub struct LastErrorRecord {
    pub error: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LastBreakResponse {
    pub reason: String,
    pub paused: bool,
}

/// Thread-safe wrapper around [`HostSession`].
#[derive(Debug)]
pub struct ControlPlane {
    inner: Mutex<HostSession>,
    last_error: Mutex<Option<LastErrorRecord>>,
}

impl ControlPlane {
    #[must_use]
    pub fn new(model: ModelId, with_border: bool) -> Self {
        trace::init_from_env();
        Self {
            inner: Mutex::new(HostSession::new(model, with_border)),
            last_error: Mutex::new(None),
        }
    }

    pub fn with_session(session: HostSession) -> Self {
        trace::init_from_env();
        Self {
            inner: Mutex::new(session),
            last_error: Mutex::new(None),
        }
    }

    pub fn record_error(&self, err: &ApiError) {
        let at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis().min(u64::MAX as u128) as u64);
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(LastErrorRecord {
                error: err.to_string(),
                status: err.status_code(),
                at_unix_ms,
            });
        }
    }

    pub fn last_error(&self) -> LastErrorRecord {
        self.last_error
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or(LastErrorRecord {
                error: String::new(),
                status: 0,
                at_unix_ms: None,
            })
    }

    fn with_session_mut<R>(
        &self,
        f: impl FnOnce(&mut HostSession) -> ApiResult<R>,
    ) -> ApiResult<R> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| ApiError::Message("session lock poisoned".into()))?;
        f(&mut guard)
    }

    fn with_session_ref<R>(&self, f: impl FnOnce(&HostSession) -> ApiResult<R>) -> ApiResult<R> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| ApiError::Message("session lock poisoned".into()))?;
        f(&guard)
    }

    pub fn health(&self) -> ApiResult<HealthResponse> {
        self.with_session_ref(|s| {
            Ok(HealthResponse {
                ok: true,
                model: model_slug(s.model()),
                has_machine: s.has_machine(),
                status: s.status().to_string(),
            })
        })
    }

    pub fn inspect_json(&self) -> ApiResult<String> {
        self.with_session_ref(|s| Ok(s.inspect_json()?))
    }

    pub fn framebuffer_meta(&self) -> ApiResult<FramebufferMeta> {
        self.with_session_ref(|s| Ok(FramebufferMeta::from_session(s)))
    }

    pub fn video_meta(&self) -> ApiResult<FramebufferMeta> {
        self.framebuffer_meta()
    }

    pub fn apply_config(&self, config: &UserMachineConfig) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.apply_user_config(config)?;
            Ok(())
        })
    }

    pub fn set_key(&self, row: usize, bit: u8, pressed: bool) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.set_key(row, bit, pressed)?;
            Ok(())
        })
    }

    pub fn clear_keys(&self) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.clear_keys()?;
            Ok(())
        })
    }

    pub fn set_joystick(&self, mask: u8) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.set_joystick(mask)?;
            Ok(())
        })
    }

    pub fn clear_joystick(&self) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.clear_joystick()?;
            Ok(())
        })
    }

    pub fn last_break(&self) -> ApiResult<LastBreakResponse> {
        self.with_session_ref(|s| {
            let reason = s.last_break_reason()?;
            Ok(LastBreakResponse {
                reason: format_break_reason(reason),
                paused: s.paused(),
            })
        })
    }

    pub fn framebuffer_rgba(&self) -> ApiResult<Vec<u8>> {
        self.with_session_ref(|s| Ok(s.framebuffer().to_vec()))
    }

    pub fn framebuffer_png(&self) -> ApiResult<Vec<u8>> {
        self.with_session_ref(encode_framebuffer_png)
    }

    pub fn set_model(&self, model: &str) -> ApiResult<()> {
        let model = parse_model_slug(model)?;
        self.with_session_mut(|s| {
            s.select_model(model)?;
            Ok(())
        })
    }

    pub fn autoload_model(&self, model: ModelId) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.select_model(model)?;
            Ok(())
        })
    }

    pub fn load_rom_bytes(&self, rom: &[u8]) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.load_rom_bytes(rom)?;
            Ok(())
        })
    }

    pub fn load_rom_path(&self, path: &Path) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.load_rom_path(path)?;
            Ok(())
        })
    }

    pub fn load_snapshot(&self, path: &Path) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.load_snapshot(path)?;
            Ok(())
        })
    }

    pub fn load_rzx(&self, path: &Path) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.load_rzx(path)?;
            Ok(())
        })
    }

    pub fn load_dsk(&self, path: &Path) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.load_dsk(path)?;
            Ok(())
        })
    }

    pub fn load_trd(&self, path: &Path) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.load_trd(path)?;
            Ok(())
        })
    }

    pub fn reset(&self) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.reset()?;
            Ok(())
        })
    }

    pub fn set_running(&self, running: bool) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.set_running(running);
            Ok(())
        })
    }

    pub fn run_frames(&self, frames: u32) -> ApiResult<RunResponse> {
        self.with_session_mut(|s| {
            let reason = s.run_frames(frames)?;
            Ok(RunResponse {
                frames,
                break_reason: format!("{reason:?}"),
                paused: s.paused(),
            })
        })
    }

    pub fn step(&self, count: u32) -> ApiResult<()> {
        self.with_session_mut(|s| {
            for _ in 0..count.max(1) {
                s.step()?;
            }
            Ok(())
        })
    }

    pub fn run_until_break(&self, max_insns: u32) -> ApiResult<RunUntilResponse> {
        self.with_session_mut(|s| {
            let reason = s.run_until_break(max_insns)?;
            Ok(RunUntilResponse {
                break_reason: format!("{reason:?}"),
                paused: s.paused(),
            })
        })
    }

    pub fn tape_open(&self, path: &Path) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.open_tape(path)?;
            Ok(())
        })
    }

    pub fn tape_play(&self) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.play_tape()?;
            Ok(())
        })
    }

    pub fn tape_pause(&self) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.pause_tape()?;
            Ok(())
        })
    }

    pub fn tape_rewind(&self) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.rewind_tape()?;
            Ok(())
        })
    }

    pub fn tape_load_options(&self, opts: TapeLoadOptions) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.set_tape_load_options(opts)?;
            Ok(())
        })
    }

    pub fn type_load(&self, with_code: bool, warmup: u32, max: u32) -> ApiResult<TypeLoadResponse> {
        self.with_session_mut(|s| {
            let result = s.type_load(with_code, warmup, max)?;
            Ok(TypeLoadResponse {
                load_ok: result.load_ok,
                attr_mark: result.attr_mark,
                inspect: s.inspect_json()?,
            })
        })
    }

    pub fn peek(&self, addr: u16, len: u16) -> ApiResult<String> {
        self.with_session_ref(|s| Ok(s.hexdump(addr, len)?))
    }

    pub fn poke(&self, addr: u16, value: u8) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.poke(addr, value)?;
            Ok(())
        })
    }

    pub fn disasm(&self, addr: Option<u16>, count: usize) -> ApiResult<String> {
        self.with_session_ref(|s| Ok(s.disasm(addr, count)?))
    }

    pub fn list_breakpoints(&self) -> ApiResult<Vec<u16>> {
        self.with_session_ref(|s| Ok(s.list_pc_breakpoints()?))
    }

    pub fn add_breakpoint(&self, pc: u16) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.add_breakpoint(pc)?;
            Ok(())
        })
    }

    pub fn remove_breakpoint(&self, pc: u16) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.remove_breakpoint(pc)?;
            Ok(())
        })
    }

    pub fn add_mem_watch(&self, watch: Watch) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.add_mem_watch(watch)?;
            Ok(())
        })
    }

    pub fn list_watches(&self) -> ApiResult<WatchesResponse> {
        self.with_session_ref(|s| {
            let (mem, port) = s.list_watches()?;
            Ok(WatchesResponse {
                mem: mem.into_iter().map(WatchSpec::from).collect(),
                port: port.into_iter().map(WatchSpec::from).collect(),
            })
        })
    }

    pub fn clear_breakpoints(&self) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.clear_breakpoints()?;
            Ok(())
        })
    }

    pub fn set_border(&self, with_border: bool) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.set_border(with_border);
            Ok(())
        })
    }

    pub fn trace_categories(&self) -> ApiResult<String> {
        let bits = trace::categories().bits();
        Ok(bits.to_string())
    }

    pub fn set_trace_categories(&self, list: &str) -> ApiResult<()> {
        trace::enable(Category::parse_list(list));
        Ok(())
    }

    pub fn trace_dump(&self, format: TraceFormat, last: Option<usize>) -> ApiResult<String> {
        Ok(match format {
            TraceFormat::Json => trace::dump_json(),
            TraceFormat::Ndjson => trace::dump_ndjson(),
            TraceFormat::Text => {
                if let Some(n) = last {
                    trace::dump_filtered(DumpFilter {
                        last_n: Some(n),
                        ..DumpFilter::default()
                    })
                } else {
                    trace::dump_string()
                }
            }
        })
    }

    pub fn trace_clear(&self) -> ApiResult<()> {
        trace::clear();
        Ok(())
    }

    pub fn rom_setup(&self) -> ApiResult<String> {
        self.with_session_ref(|s| {
            let setup = rom_setup_json(s.model(), &model_rom_paths_snapshot());
            serde_json::to_string(&setup).map_err(|e| ApiError::Message(e.to_string()))
        })
    }

    pub fn insert_dck(&self, path: &Path) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.insert_dck(path)?;
            Ok(())
        })
    }

    pub fn eject_dck(&self) -> ApiResult<()> {
        self.with_session_mut(|s| {
            s.eject_dck()?;
            Ok(())
        })
    }

    pub fn status(&self) -> ApiResult<StatusResponse> {
        self.with_session_ref(|s| {
            Ok(StatusResponse {
                model: model_slug(s.model()),
                has_machine: s.has_machine(),
                running: s.running(),
                paused: s.paused(),
                with_border: s.with_border(),
                status: s.status().to_string(),
                tape_playing: s.tape_playing(),
                has_tape: s.has_tape(),
            })
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub model: String,
    pub has_machine: bool,
    pub status: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunResponse {
    pub frames: u32,
    pub break_reason: String,
    pub paused: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunUntilResponse {
    pub break_reason: String,
    pub paused: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct TypeLoadResponse {
    pub load_ok: bool,
    pub attr_mark: Option<u8>,
    pub inspect: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct WatchSpec {
    pub addr: u16,
    pub read: bool,
    pub write: bool,
}

impl From<machine::Watch> for WatchSpec {
    fn from(w: machine::Watch) -> Self {
        Self {
            addr: w.addr,
            read: w.read,
            write: w.write,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct WatchesResponse {
    pub mem: Vec<WatchSpec>,
    pub port: Vec<WatchSpec>,
}

fn format_break_reason(reason: BreakReason) -> String {
    match reason {
        BreakReason::None => "none".into(),
        BreakReason::Pc(pc) => format!("pc:{pc:04X}"),
        BreakReason::Mem { addr, write, value } => {
            format!(
                "mem:{addr:04X}:{}={value:02X}",
                if write { "w" } else { "r" }
            )
        }
        BreakReason::Port { port, write, value } => {
            format!(
                "port:{port:04X}:{}={value:02X}",
                if write { "w" } else { "r" }
            )
        }
        BreakReason::Halt => "halt".into(),
        BreakReason::Budget => "budget".into(),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct StatusResponse {
    pub model: String,
    pub has_machine: bool,
    pub running: bool,
    pub paused: bool,
    pub with_border: bool,
    pub status: String,
    pub tape_playing: bool,
    pub has_tape: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine::Model;

    fn rom48() -> Option<Vec<u8>> {
        machine::resolve_rom_path(Model::Spectrum48).and_then(|path| std::fs::read(path).ok())
    }

    #[test]
    fn health_and_inspect_after_rom_load() {
        let Some(rom) = rom48() else {
            eprintln!("skip: Spectrum 48 ROM missing");
            return;
        };
        let plane = ControlPlane::new(ModelId::Spectrum48, false);
        {
            let mut s = plane.inner.lock().expect("lock");
            s.load_rom_bytes(&rom).expect("rom");
        }
        let health = plane.health().expect("health");
        assert!(health.has_machine);
        let inspect = plane.inspect_json().expect("inspect");
        assert!(inspect.contains("\"pc\""));
    }

    #[test]
    fn run_frames_and_framebuffer_dims() {
        let Some(rom) = rom48() else {
            eprintln!("skip: Spectrum 48 ROM missing");
            return;
        };
        let plane = ControlPlane::new(ModelId::Spectrum48, false);
        {
            let mut s = plane.inner.lock().expect("lock");
            s.load_rom_bytes(&rom).expect("rom");
        }
        plane.run_frames(1).expect("run");
        let meta = plane.framebuffer_meta().expect("meta");
        assert_eq!(meta.width, 256);
        assert_eq!(meta.height, 192);
        assert!(!meta.hires);
        let png = plane.framebuffer_png().expect("png");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn parse_model_slug_rejects_unknown() {
        assert!(parse_model_slug("not-a-model").is_err());
    }

    #[test]
    fn parse_model_slug_accepts_canonical_timex_aliases() {
        assert_eq!(
            parse_model_slug("timex_ts2068").expect("ts2068"),
            ModelId::TimexTS2068
        );
        assert_eq!(
            parse_model_slug("timex_tc2048").expect("tc2048"),
            ModelId::TimexTC2048
        );
    }

    #[test]
    fn server_config_rejects_public_bind() {
        assert!(ServerConfig::validate_bind_host("0.0.0.0").is_err());
        assert!(ServerConfig::validate_bind_host("127.0.0.1").is_ok());
    }

    #[test]
    fn server_config_requires_token_or_insecure() {
        let no_token = ServerConfig {
            host: "127.0.0.1".into(),
            port: 17_384,
            token: None,
            insecure: false,
        };
        assert!(no_token.validate_auth_config().is_err());
        let insecure = ServerConfig {
            insecure: true,
            ..no_token.clone()
        };
        assert!(insecure.validate_auth_config().is_ok());
        let with_token = ServerConfig {
            token: Some("secret".into()),
            insecure: false,
            ..no_token
        };
        assert!(with_token.validate_auth_config().is_ok());
    }

    #[test]
    fn last_error_records_failures() {
        let plane = ControlPlane::new(ModelId::Spectrum48, false);
        assert!(plane.last_error().error.is_empty());
        plane.record_error(&ApiError::BadRequest("test".into()));
        let last = plane.last_error();
        assert!(last.error.contains("test"));
        assert_eq!(last.status, 400);
    }

    #[test]
    fn set_key_requires_machine() {
        let plane = ControlPlane::new(ModelId::Spectrum48, false);
        assert!(plane.set_key(0, 0, true).is_err());
    }

    #[test]
    fn set_joystick_requires_machine() {
        let plane = ControlPlane::new(ModelId::Spectrum48, false);
        assert!(plane.set_joystick(0x11).is_err());
        assert!(plane.clear_joystick().is_err());
    }

    #[test]
    fn set_joystick_round_trip() {
        let Some(rom) = rom48() else {
            eprintln!("skip: Spectrum 48 ROM missing");
            return;
        };
        let plane = ControlPlane::new(ModelId::Spectrum48, false);
        {
            let mut s = plane.inner.lock().expect("lock");
            s.load_rom_bytes(&rom).expect("rom");
        }
        plane.set_joystick(0x11).expect("set");
        plane.clear_joystick().expect("clear");
    }
}
