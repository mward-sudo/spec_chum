//! Optional `ZXInfo` online tape title enrichment (#373).
//!
//! Offline catalogue / filename resolution stays in `formats::media_identity`.
//! This module owns disk cache + HTTPS `filecheck` (never blocks tape open).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use formats::MediaTitleSource;
use serde::{Deserialize, Serialize};

const ZXINFO_FILECHECK: &str = "https://api.zxinfo.dk/v3/filecheck";
const HTTP_TIMEOUT: Duration = Duration::from_secs(4);

/// Spec Chum User-Agent for `ZXInfo` (identify the client; avoid generic crawlers).
#[must_use]
pub fn user_agent() -> String {
    format!(
        "SpecChum/{} (+https://github.com/mward-sudo/spec_chum)",
        env!("CARGO_PKG_VERSION")
    )
}

/// One cached `ZXInfo` / prior-hit entry (metadata only — never tape bytes).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedTitle {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct TitleCacheFile {
    #[serde(default)]
    entries: BTreeMap<String, CachedTitle>,
}

/// On-disk cache path (`…/spec-chum/zxinfo-titles.json`), or `SPEC_CHUM_ZXINFO_CACHE_PATH`.
#[must_use]
pub fn default_cache_path() -> PathBuf {
    if let Ok(p) = std::env::var("SPEC_CHUM_ZXINFO_CACHE_PATH") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    directories::ProjectDirs::from("dev", "SpecChum", "spec-chum").map_or_else(
        || PathBuf::from("zxinfo-titles.json"),
        |d| d.cache_dir().join("zxinfo-titles.json"),
    )
}

/// Read a cached title for `sha512_hex` (128 lowercase hex).
#[must_use]
pub fn cache_get(path: &Path, sha512_hex: &str) -> Option<CachedTitle> {
    let bytes = fs::read(path).ok()?;
    let file: TitleCacheFile = serde_json::from_slice(&bytes).ok()?;
    file.entries.get(sha512_hex).cloned()
}

/// Persist `sha512 → title` (best-effort atomic write).
pub fn cache_put(
    path: &Path,
    sha512_hex: &str,
    title: &str,
    entry_id: Option<&str>,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut file = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => TitleCacheFile::default(),
    };
    file.entries.insert(
        sha512_hex.to_string(),
        CachedTitle {
            title: title.to_string(),
            entry_id: entry_id.map(str::to_owned),
        },
    );
    let json = serde_json::to_vec_pretty(&file)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Successful `ZXInfo` `filecheck` hit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZxinfoHit {
    pub title: String,
    pub entry_id: Option<String>,
}

#[derive(Deserialize)]
struct ZxinfoFilecheckBody {
    title: Option<String>,
    entry_id: Option<String>,
}

/// HTTPS GET `ZXInfo` filecheck. Returns `Ok(None)` on 404 / empty / soft failures.
pub fn fetch_zxinfo_title(sha512_hex: &str) -> Result<Option<ZxinfoHit>, String> {
    if sha512_hex.len() != 128 || !sha512_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid sha512 hex".into());
    }
    let url = format!("{ZXINFO_FILECHECK}/{sha512_hex}");
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .user_agent(user_agent())
        .build()
        .new_agent();
    let resp = match agent.get(&url).call() {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let status = resp.status();
    let code = status.as_u16();
    if code == 404 {
        return Ok(None);
    }
    if !(200..300).contains(&code) {
        return Err(format!("ZXInfo HTTP {code}"));
    }
    let body: ZxinfoFilecheckBody = resp.into_body().read_json().map_err(|e| e.to_string())?;
    let title = body
        .title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    Ok(title.map(|title| ZxinfoHit {
        title,
        entry_id: body.entry_id,
    }))
}

/// Enrichment result for the session inbox (cache or live hit).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleEnrichment {
    pub title: String,
    pub source: MediaTitleSource,
    pub entry_id: Option<String>,
}

/// Resolve online layers only (caller already knows local catalogue missed).
///
/// Order: disk cache → optional `fetch` → `None` (keep filename).
pub fn enrich_after_filename(
    sha512_hex: &str,
    cache_path: &Path,
    online_enabled: bool,
    fetch: impl FnOnce(&str) -> Result<Option<ZxinfoHit>, String>,
) -> Option<TitleEnrichment> {
    if let Some(hit) = cache_get(cache_path, sha512_hex) {
        return Some(TitleEnrichment {
            title: hit.title,
            source: MediaTitleSource::Cached,
            entry_id: hit.entry_id,
        });
    }
    if !online_enabled {
        return None;
    }
    match fetch(sha512_hex) {
        Ok(Some(hit)) => {
            let _ = cache_put(cache_path, sha512_hex, &hit.title, hit.entry_id.as_deref());
            Some(TitleEnrichment {
                title: hit.title,
                source: MediaTitleSource::Online,
                entry_id: hit.entry_id,
            })
        }
        Ok(None) | Err(_) => None,
    }
}

/// Full display resolve order for tests: catalogue → cache/online → filename.
#[must_use]
pub fn resolve_display_title(
    sha512_hex: &str,
    filename: &str,
    online_enabled: bool,
    catalogue: impl FnOnce(&str) -> Option<&'static str>,
    cache_path: &Path,
    fetch: impl FnOnce(&str) -> Result<Option<ZxinfoHit>, String>,
) -> (String, MediaTitleSource) {
    if let Some(title) = catalogue(sha512_hex) {
        return (title.to_string(), MediaTitleSource::LocalCatalogue);
    }
    if let Some(enrich) = enrich_after_filename(sha512_hex, cache_path, online_enabled, fetch) {
        return (enrich.title, enrich.source);
    }
    (filename.to_string(), MediaTitleSource::Filename)
}

/// Background lookup: cache then `ZXInfo`; returns enrichment or `None`.
pub fn lookup_online(sha512_hex: &str, cache_path: &Path) -> Option<TitleEnrichment> {
    enrich_after_filename(sha512_hex, cache_path, true, fetch_zxinfo_title)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_cache(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!("spec-chum-zxinfo-{name}-{nanos}.json"))
    }

    #[test]
    fn resolve_order_catalogue_beats_online() {
        let path = temp_cache("cat");
        let fetches = AtomicUsize::new(0);
        let (title, source) = resolve_display_title(
            "aa",
            "noise.tap",
            true,
            |_| Some("Catalogue Hit"),
            &path,
            |_| {
                fetches.fetch_add(1, Ordering::SeqCst);
                Ok(Some(ZxinfoHit {
                    title: "Online".into(),
                    entry_id: None,
                }))
            },
        );
        assert_eq!(title, "Catalogue Hit");
        assert_eq!(source, MediaTitleSource::LocalCatalogue);
        assert_eq!(fetches.load(Ordering::SeqCst), 0);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn resolve_order_cache_before_network() {
        let path = temp_cache("cache");
        cache_put(&path, "bb", "Cached Title", Some("1")).expect("cache");
        let fetches = AtomicUsize::new(0);
        let (title, source) = resolve_display_title(
            "bb",
            "noise.tap",
            true,
            |_| None,
            &path,
            |_| {
                fetches.fetch_add(1, Ordering::SeqCst);
                Ok(Some(ZxinfoHit {
                    title: "Online".into(),
                    entry_id: None,
                }))
            },
        );
        assert_eq!(title, "Cached Title");
        assert_eq!(source, MediaTitleSource::Cached);
        assert_eq!(fetches.load(Ordering::SeqCst), 0);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn resolve_order_online_then_filename() {
        let path = temp_cache("online");
        let (title, source) = resolve_display_title(
            "cc",
            "fallback.tap",
            true,
            |_| None,
            &path,
            |_| {
                Ok(Some(ZxinfoHit {
                    title: "Jetpac".into(),
                    entry_id: Some("0009362".into()),
                }))
            },
        );
        assert_eq!(title, "Jetpac");
        assert_eq!(source, MediaTitleSource::Online);
        assert_eq!(
            cache_get(&path, "cc").map(|c| c.title),
            Some("Jetpac".into())
        );

        let path2 = temp_cache("miss");
        let (title2, source2) =
            resolve_display_title("dd", "fallback.tap", true, |_| None, &path2, |_| Ok(None));
        assert_eq!(title2, "fallback.tap");
        assert_eq!(source2, MediaTitleSource::Filename);

        let (title3, source3) = resolve_display_title(
            "ee",
            "offline.tap",
            false,
            |_| None,
            &path2,
            |_| panic!("must not fetch when disabled"),
        );
        assert_eq!(title3, "offline.tap");
        assert_eq!(source3, MediaTitleSource::Filename);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&path2);
    }

    #[test]
    fn user_agent_identifies_spec_chum() {
        let ua = user_agent();
        assert!(ua.starts_with("SpecChum/"));
        assert!(ua.contains("github.com/mward-sudo/spec_chum"));
    }
}
