//! Trace dump / filter / append-file helpers.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;

use crate::category::Category;
use crate::event::TraceEvent;
use crate::{categories, snapshot};

fn describe_categories(c: Category) -> String {
    let mut parts = Vec::new();
    if c.contains(Category::CPU) {
        parts.push("cpu");
    }
    if c.contains(Category::BUS) {
        parts.push("bus");
    }
    if c.contains(Category::TAPE) {
        parts.push("tape");
    }
    if c.contains(Category::ULA) {
        parts.push("ula");
    }
    if c.contains(Category::MACHINE) {
        parts.push("machine");
    }
    if c.contains(Category::AY) {
        parts.push("ay");
    }
    if c.contains(Category::DISK) {
        parts.push("disk");
    }
    if c.contains(Category::MEM) {
        parts.push("mem");
    }
    if parts.is_empty() {
        "none".into()
    } else {
        parts.join(",")
    }
}

/// Format the ring as text (one event per line) with a short header.
#[must_use]
pub fn dump_string() -> String {
    let cats = categories();
    let events = snapshot();
    let mut out = String::with_capacity(events.len().saturating_mul(96) + 128);
    out.push_str(&format!(
        "# spec_chum trace dump events={} categories=0x{:x} ({})\n",
        events.len(),
        cats.bits(),
        describe_categories(cats)
    ));
    for ev in &events {
        out.push_str(&ev.to_string());
        out.push('\n');
    }
    out
}

/// Optional filters for [`dump_filtered`].
#[derive(Clone, Copy, Debug, Default)]
pub struct DumpFilter {
    /// If non-zero, keep events whose category intersects this mask.
    pub category: Category,
    pub t_min: Option<u64>,
    pub t_max: Option<u64>,
    pub pc_min: Option<u16>,
    pub pc_max: Option<u16>,
    pub last_n: Option<usize>,
}

#[must_use]
pub fn dump_filtered(filter: DumpFilter) -> String {
    let mut events = snapshot();
    if filter.category.bits() != 0 {
        events.retain(|e| e.kind.category().bits() & filter.category.bits() != 0);
    }
    if let Some(t0) = filter.t_min {
        events.retain(|e| e.t >= t0);
    }
    if let Some(t1) = filter.t_max {
        events.retain(|e| e.t <= t1);
    }
    if filter.pc_min.is_some() || filter.pc_max.is_some() {
        events.retain(|e| {
            let Some(pc) = e.kind.pc() else {
                return false;
            };
            if filter.pc_min.is_some_and(|lo| pc < lo) {
                return false;
            }
            if filter.pc_max.is_some_and(|hi| pc > hi) {
                return false;
            }
            true
        });
    }
    if let Some(n) = filter.last_n {
        let skip = events.len().saturating_sub(n);
        events.drain(..skip);
    }
    let mut out = String::with_capacity(events.len().saturating_mul(96) + 64);
    out.push_str(&format!(
        "# spec_chum trace dump events={} (filtered)\n",
        events.len()
    ));
    for ev in &events {
        out.push_str(&ev.to_string());
        out.push('\n');
    }
    out
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out
}

fn event_json(ev: &TraceEvent) -> String {
    let cat = describe_categories(ev.kind.category());
    let text = json_escape(&ev.kind.to_string());
    let pc = ev.kind.pc().map_or("null".into(), |p| p.to_string());
    format!(
        "{{\"seq\":{},\"t\":{},\"cat\":\"{cat}\",\"pc\":{pc},\"text\":\"{text}\"}}",
        ev.seq, ev.t
    )
}

/// JSON array of ring events (hand-rolled, no serde).
#[must_use]
pub fn dump_json() -> String {
    let events = snapshot();
    let mut out = String::from("[\n");
    for (i, ev) in events.iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        out.push_str(&event_json(ev));
    }
    out.push_str("\n]\n");
    out
}

/// One JSON object per line.
#[must_use]
pub fn dump_ndjson() -> String {
    let mut out = String::new();
    for ev in snapshot() {
        out.push_str(&event_json(&ev));
        out.push('\n');
    }
    out
}

struct AppendSink {
    writer: Option<std::io::BufWriter<File>>,
    decided: bool,
    /// Last I/O error from append write/flush (cleared on successful flush).
    last_error: Option<String>,
}

static APPEND: Mutex<AppendSink> = Mutex::new(AppendSink {
    writer: None,
    decided: false,
    last_error: None,
});

pub(crate) fn maybe_append(ev: &TraceEvent) {
    let Ok(mut sink) = APPEND.lock() else {
        return;
    };
    if !sink.decided {
        sink.decided = true;
        let flag = std::env::var("SPEC_CHUM_TRACE_APPEND").is_ok_and(|v| {
            let t = v.trim();
            t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
        });
        if flag {
            match std::env::var("SPEC_CHUM_TRACE_FILE") {
                Ok(path) if !path.is_empty() => {
                    match std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                    {
                        Ok(f) => sink.writer = Some(std::io::BufWriter::new(f)),
                        Err(e) => {
                            sink.last_error =
                                Some(format!("open SPEC_CHUM_TRACE_FILE ({path}): {e}"));
                        }
                    }
                }
                Ok(_) | Err(_) => {}
            }
        }
    }
    if let Some(w) = sink.writer.as_mut() {
        if let Err(e) = writeln!(w, "{ev}") {
            sink.last_error = Some(format!("append write: {e}"));
        }
    }
}

/// Flush append-mode output so short CLI runs are not left empty.
///
/// Returns `Err` if an earlier append write failed or the final flush fails.
pub fn flush_append() -> io::Result<()> {
    let Ok(mut sink) = APPEND.lock() else {
        return Err(io::Error::other("trace append lock poisoned"));
    };
    if let Some(err) = sink.last_error.take() {
        return Err(io::Error::other(err));
    }
    if let Some(w) = sink.writer.as_mut() {
        w.flush()?;
    }
    Ok(())
}

#[cfg(test)]
pub fn reset_append_sink_for_tests() {
    if let Ok(mut sink) = APPEND.lock() {
        *sink = AppendSink {
            writer: None,
            decided: false,
            last_error: None,
        };
    }
}

#[cfg(test)]
pub fn configure_append_file_for_tests(path: &Path) {
    reset_append_sink_for_tests();
    if let Ok(mut sink) = APPEND.lock() {
        if let Ok(f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            sink.writer = Some(std::io::BufWriter::new(f));
        }
        sink.decided = true;
    }
}

/// Write dump to `w`.
pub fn dump_to_writer(mut w: impl Write) -> io::Result<()> {
    w.write_all(dump_string().as_bytes())
}

/// Write dump to a file path (creates/truncates).
pub fn dump_to_file(path: impl AsRef<Path>) -> io::Result<()> {
    let mut f = File::create(path)?;
    dump_to_writer(&mut f)
}

/// Dump to stderr (agents / CI failure path).
pub fn dump_to_stderr() {
    let _ = dump_to_writer(io::stderr());
}

/// If `SPEC_CHUM_TRACE_FILE` is set, write the dump there.
pub fn dump_to_env_file() -> io::Result<Option<std::path::PathBuf>> {
    let Ok(path) = std::env::var("SPEC_CHUM_TRACE_FILE") else {
        return Ok(None);
    };
    if path.is_empty() {
        return Ok(None);
    }
    let p = std::path::PathBuf::from(path);
    dump_to_file(&p)?;
    Ok(Some(p))
}
