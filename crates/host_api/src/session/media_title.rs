//! Tape identity and asynchronous title enrichment for [`super::HostSession`].

use std::path::Path;
use std::sync::Arc;
use std::thread;

use formats::MediaTitleSource;

use super::HostSession;

/// Background title enrichment waiting to be applied on the host thread.
#[derive(Clone, Debug)]
pub(super) struct PendingMediaTitle {
    pub(super) generation: u64,
    pub(super) sha512_hex: String,
    pub(super) title: String,
    pub(super) source: MediaTitleSource,
}

impl HostSession {
    /// Human title for chrome / agent status (catalogue, cache, online, or filename).
    /// Returns `None` when no tape is inserted. Applies completed enrichment first.
    #[must_use]
    pub fn media_title(&mut self) -> Option<&str> {
        self.apply_pending_media_title();
        self.has_tape()
            .then_some(self.media_title.as_deref())
            .flatten()
    }

    /// SHA-512 hex of the inserted tape file, when known.
    #[must_use]
    pub fn media_sha512(&mut self) -> Option<&str> {
        self.apply_pending_media_title();
        self.has_tape()
            .then_some(self.media_sha512.as_deref())
            .flatten()
    }

    /// Opt-in `ZXInfo` title lookup by hash. Disabled by default for privacy.
    pub fn set_online_tape_titles(&mut self, enabled: bool) {
        self.online_tape_titles = enabled;
    }

    /// Whether online `ZXInfo` title lookup is enabled.
    #[must_use]
    pub fn online_tape_titles(&self) -> bool {
        self.online_tape_titles
    }

    /// Source of the inserted tape's current title, when available.
    #[must_use]
    pub fn media_title_source(&mut self) -> Option<MediaTitleSource> {
        self.apply_pending_media_title();
        self.has_tape().then_some(self.media_title_source).flatten()
    }

    /// Resolve and store tape identity from a path using the offline catalogue or filename.
    pub fn set_media_identity_from_path(&mut self, path: &Path) {
        self.media_path = Some(path.to_path_buf());
        if let Ok(id) = formats::identify_path(path) {
            self.install_media_identity(id.display_title, Some(id.sha512_hex), id.source);
            return;
        }
        let title = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| path.display().to_string());
        self.install_media_identity(title, None, MediaTitleSource::Filename);
    }

    /// Resolve identity from bytes already read for open, avoiding a second read.
    pub fn set_media_identity_from_bytes(&mut self, bytes: &[u8], path: &Path) {
        self.media_path = Some(path.to_path_buf());
        let id = formats::identify_bytes(bytes, path);
        self.install_media_identity(id.display_title, Some(id.sha512_hex), id.source);
    }

    /// Return known title metadata only for the exact path that produced the active tape.
    pub(crate) fn media_identity_for_path(
        &mut self,
        path: &Path,
    ) -> Option<(&str, MediaTitleSource)> {
        self.apply_pending_media_title();
        if !self.has_tape() || self.media_path.as_deref() != Some(path) {
            return None;
        }
        Some((self.media_title.as_deref()?, self.media_title_source?))
    }

    fn install_media_identity(
        &mut self,
        title: String,
        sha512_hex: Option<String>,
        source: MediaTitleSource,
    ) {
        self.media_title_generation = self.media_title_generation.wrapping_add(1);
        *self.pending_media_title.lock() = None;
        self.media_title = Some(title);
        self.media_sha512 = sha512_hex;
        self.media_title_source = Some(source);
        self.maybe_spawn_online_lookup();
    }

    pub(super) fn clear_media_identity(&mut self) {
        self.media_title_generation = self.media_title_generation.wrapping_add(1);
        *self.pending_media_title.lock() = None;
        self.media_title = None;
        self.media_sha512 = None;
        self.media_title_source = None;
        self.media_path = None;
    }

    fn maybe_spawn_online_lookup(&mut self) {
        if !self.online_tape_titles || self.media_title_source != Some(MediaTitleSource::Filename) {
            return;
        }
        let Some(sha) = self.media_sha512.clone() else {
            return;
        };
        let generation = self.media_title_generation;
        let inbox = Arc::clone(&self.pending_media_title);
        let cache_path = crate::media_title_lookup::default_cache_path();
        thread::spawn(move || {
            let Some(enrich) = crate::media_title_lookup::lookup_online(&sha, &cache_path) else {
                return;
            };
            *inbox.lock() = Some(PendingMediaTitle {
                generation,
                sha512_hex: sha,
                title: enrich.title,
                source: enrich.source,
            });
        });
    }

    /// Apply a completed background title enrichment if it still matches this tape.
    pub fn apply_pending_media_title(&mut self) {
        let Some(pending) = self.pending_media_title.lock().take() else {
            return;
        };
        if pending.generation != self.media_title_generation
            || self.media_sha512.as_deref() != Some(pending.sha512_hex.as_str())
            || !self.has_tape()
        {
            return;
        }
        if let Some(old) = self.media_title.replace(pending.title.clone()) {
            if self.status.contains(&old) {
                self.status = self.status.replacen(&old, &pending.title, 1);
            }
        }
        self.media_title_source = Some(pending.source);
    }
}
