# Tape identity (content hash)

Spec Chum resolves a **display title** for inserted TAP/TZX files from a
content hash, falling back to the filesystem basename when unknown.

Tracked in [#366](https://github.com/mward-sudo/spec_chum/issues/366) (offline)
and [#373](https://github.com/mward-sudo/spec_chum/issues/373) (optional online).

## Hash algorithm

**SHA-512 of the raw file bytes** (entire file on disk; no header stripping or
payload normalisation). Digests are **lowercase hex** (128 characters).

This matches the [ZXInfo API v3](https://api.zxinfo.dk/v3/) contract:

```http
GET /filecheck/{hash}
```

where `{hash}` is MD5 (32 hex chars) or SHA-512 (128 hex chars). Spec Chum
uses SHA-512 only.

## Lookup order

| Layer | Role |
| --- | --- |
| **Embedded local catalogue** | Metadata-only `hash → title` in `formats::media_identity` (offline, no network). Ships titles for redistributable test fixtures only — **never** copyrighted tape images. |
| **On-disk cache** | Prior ZXInfo / enrichment hits under the app cache dir (`zxinfo-titles.json`). |
| **ZXInfo (opt-in)** | Live `GET https://api.zxinfo.dk/v3/filecheck/{sha512}` when the user enables online titles. |
| **Filename fallback** | On miss / I/O / network failure: `path.file_name()` (same idea as macOS `lastPathComponent`). |

Core emulation never depends on network or catalogue hits. Tape **Open** / **Play**
never block on HTTP.

## Online ZXInfo (#373)

### Preference

- Pref key: `online_tape_titles` in `UiPreferences` / UserDefaults `specChum.onlineTapeTitles`.
- **Default: off** (privacy). Enable in SpecChumMac **Settings → Tape** or egui **Tape** menu:
  “Look up tape titles online (ZXInfo)”.
- When enabled, Spec Chum sends **only** the SHA-512 of the opened file to
  `api.zxinfo.dk` (not the path, not file bytes). A hit reveals which known
  release was opened.

### HTTP

```http
GET https://api.zxinfo.dk/v3/filecheck/{sha512}
User-Agent: SpecChum/<version> (+https://github.com/mward-sudo/spec_chum)
```

- **200** JSON with at least `title` (and usually `entry_id`).
- **404** → keep filename.
- Timeout / offline / 4xx/5xx → keep filename (short global timeout; no retry storm).

Implementation: `host_api::media_title_lookup` (`ureq` + rustls). HTTP is **not**
in the `formats` crate.

### UX

1. Open tape → show local catalogue title, else filename immediately.
2. If opt-in and local miss → background cache/ZXInfo lookup.
3. On hit → update `HostSession::media_title` (status / window media / `GET /v1/status`)
   without re-opening the tape.

### Cache location

Default: platform cache dir for `dev.SpecChum.spec-chum` /
`zxinfo-titles.json` (via `directories::ProjectDirs`).

Override: `SPEC_CHUM_ZXINFO_CACHE_PATH` (useful in tests).

Cache stores metadata only (`sha512 → { title, entry_id? }`).

## Host surfaces

| Surface | Behaviour |
| --- | --- |
| **Shared** | `HostSession::media_title` / `media_sha512` set on tape open; cleared on eject; online hits apply asynchronously |
| **Agent** | `GET /v1/status` includes optional `media_title` and `media_sha512` |
| **FFI** | `sc_media_title` / `sc_media_sha512`; `sc_set_online_tape_titles` / `sc_online_tape_titles` |
| **egui** | Status text + top-bar tape label; Tape menu opt-in toggle |
| **SpecChumMac** | Status footer / window media; Settings toggle; title refreshed after background hit |

## Extending the catalogue

Add `(sha512_hex, "Human Title")` rows to `LOCAL_CATALOGUE` in
`crates/formats/src/media_identity.rs`. Prefer redistributable / fixture media
only. To compute a digest:

```bash
shasum -a 512 path/to/file.tap
```

## Non-goals

- Shipping copyrighted tape images
- Network-required title resolution
- Perfect TOSEC renaming of a user’s library
- Uploading tape bytes or paths
