//! Thin HTTP client for the Spec Chum agent debug API (#210 Phase B).

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct AgentClient {
    base: String,
    token: Option<String>,
    agent: ureq::Agent,
}

impl AgentClient {
    pub fn from_env() -> Result<Self> {
        let base = std::env::var("SPEC_CHUM_AGENT_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:17384".into());
        let token = std::env::var("SPEC_CHUM_AGENT_TOKEN").ok();
        Self::new(&base, token)
    }

    pub fn new(base: &str, token: Option<String>) -> Result<Self> {
        let base = base.trim_end_matches('/').to_string();
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .new_agent();
        Ok(Self { base, token, agent })
    }

    #[allow(dead_code)]
    pub fn health(&self) -> Result<Value> {
        self.get_json("/v1/health")
    }

    pub fn inspect_json(&self) -> Result<String> {
        let resp = self
            .agent
            .get(self.url("/v1/inspect"))
            .apply(self)
            .call()
            .context("GET /v1/inspect")?;
        resp.into_body().read_to_string().context("inspect body")
    }

    pub fn set_model(&self, model: &str) -> Result<()> {
        self.post_empty("/v1/model", serde_json::json!({ "model": model }))
    }

    pub fn load_rom(&self, path: &str) -> Result<()> {
        self.post_empty("/v1/rom", serde_json::json!({ "path": path }))
    }

    pub fn load_snapshot(&self, path: &str) -> Result<()> {
        self.post_empty("/v1/snapshot", serde_json::json!({ "path": path }))
    }

    pub fn run_frames(&self, frames: u32) -> Result<Value> {
        self.post_json("/v1/run", serde_json::json!({ "frames": frames }))
    }

    pub fn run_until(&self, max_insns: u64) -> Result<Value> {
        let max = u32::try_from(max_insns).unwrap_or(u32::MAX);
        self.post_json("/v1/run-until", serde_json::json!({ "max_insns": max }))
    }

    pub fn add_breakpoint(&self, pc: &str) -> Result<()> {
        self.post_empty("/v1/debug/breakpoints", serde_json::json!({ "pc": pc }))
    }

    pub fn add_mem_watch_write(&self, addr: &str) -> Result<()> {
        self.post_empty(
            "/v1/debug/watches",
            serde_json::json!({
                "addr": addr,
                "read": false,
                "write": true,
            }),
        )
    }

    pub fn tape_play(&self) -> Result<()> {
        self.post_empty("/v1/tape/play", serde_json::json!({}))
    }

    pub fn type_load(&self, code: bool, warmup: u32, max: u32) -> Result<Value> {
        self.post_json(
            "/v1/type-load",
            serde_json::json!({
                "code": code,
                "warmup": warmup,
                "max": max,
            }),
        )
    }

    pub fn tape_open(&self, path: &str) -> Result<()> {
        self.post_empty("/v1/tape/open", serde_json::json!({ "path": path }))
    }

    pub fn tape_load_options(&self, ear_load: bool, speed: u32) -> Result<()> {
        self.post_empty(
            "/v1/tape/load",
            serde_json::json!({
                "flash_load": !ear_load,
                "ear_load": ear_load,
                "speed": speed,
            }),
        )
    }

    pub fn peek(&self, addr: &str, len: u16) -> Result<String> {
        let resp = self
            .agent
            .get(self.url(&format!("/v1/peek?addr={}&len={len}", url_encode(addr))))
            .apply(self)
            .call()
            .context("GET /v1/peek")?;
        resp.into_body().read_to_string().context("peek body")
    }

    pub fn disasm(&self, addr: Option<&str>, count: usize) -> Result<String> {
        let path = match addr {
            Some(a) => format!("/v1/disasm?addr={}&count={count}", url_encode(a)),
            None => format!("/v1/disasm?count={count}"),
        };
        let resp = self
            .agent
            .get(self.url(&path))
            .apply(self)
            .call()
            .context("GET /v1/disasm")?;
        resp.into_body().read_to_string().context("disasm body")
    }

    pub fn dump_trace(&self, json: bool, last: Option<usize>) -> Result<String> {
        let mut path = format!("/v1/trace?format={}", if json { "json" } else { "text" });
        if let Some(n) = last {
            path.push_str(&format!("&last={n}"));
        }
        let resp = self
            .agent
            .get(self.url(&path))
            .apply(self)
            .call()
            .context("GET /v1/trace")?;
        resp.into_body().read_to_string().context("trace body")
    }

    pub fn set_trace_categories(&self, list: &str) -> Result<()> {
        let resp = self
            .agent
            .put(self.url("/v1/trace/categories"))
            .apply(self)
            .send_json(serde_json::json!({ "categories": list }))
            .context("PUT /v1/trace/categories")?;
        let status = resp.status();
        if status == 204 || status == 200 {
            Ok(())
        } else {
            let text = resp.into_body().read_to_string().unwrap_or_default();
            bail!("PUT /v1/trace/categories failed ({status}): {text}");
        }
    }

    #[allow(dead_code)]
    pub fn video_meta(&self) -> Result<Value> {
        self.get_json("/v1/video")
    }

    #[allow(dead_code)]
    pub fn last_error(&self) -> Result<Value> {
        self.get_json("/v1/errors/last")
    }

    #[allow(dead_code)]
    pub fn set_key(&self, row: usize, bit: u8, pressed: bool) -> Result<()> {
        self.post_empty(
            "/v1/keys",
            serde_json::json!({ "row": row, "bit": bit, "pressed": pressed }),
        )
    }

    #[allow(dead_code)]
    pub fn clear_keys(&self) -> Result<()> {
        self.post_empty("/v1/keys", serde_json::json!({ "clear": true }))
    }

    #[allow(dead_code)]
    pub fn last_break(&self) -> Result<Value> {
        self.get_json("/v1/debug/last-break")
    }

    #[allow(dead_code)]
    pub fn apply_config(&self, config: &Value) -> Result<()> {
        self.post_empty("/v1/config", config.clone())
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    #[allow(dead_code)]
    fn get_json(&self, path: &str) -> Result<Value> {
        let resp = self
            .agent
            .get(self.url(path))
            .apply(self)
            .call()
            .with_context(|| format!("GET {path}"))?;
        resp.into_body()
            .read_json()
            .with_context(|| format!("decode {path}"))
    }

    fn post_json(&self, path: &str, body: Value) -> Result<Value> {
        let resp = self
            .agent
            .post(self.url(path))
            .apply(self)
            .send_json(body)
            .with_context(|| format!("POST {path}"))?;
        if resp.status() == 204 {
            return Ok(Value::Null);
        }
        resp.into_body()
            .read_json()
            .with_context(|| format!("decode {path}"))
    }

    fn post_empty(&self, path: &str, body: Value) -> Result<()> {
        let resp = self
            .agent
            .post(self.url(path))
            .apply(self)
            .send_json(body)
            .with_context(|| format!("POST {path}"))?;
        let status = resp.status();
        if status == 204 || status == 200 {
            Ok(())
        } else {
            let text = resp.into_body().read_to_string().unwrap_or_default();
            bail!("POST {path} failed ({status}): {text}");
        }
    }
}

trait AuthRequest {
    fn apply(self, client: &AgentClient) -> Self;
}

impl<B> AuthRequest for ureq::RequestBuilder<B> {
    fn apply(self, client: &AgentClient) -> Self {
        if let Some(token) = &client.token {
            self.header("Authorization", format!("Bearer {token}"))
        } else {
            self
        }
    }
}

fn url_encode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '0'..='9' | 'A'..='Z' | 'a'..='z' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", u8::try_from(c).unwrap_or(0)),
        })
        .collect()
}
