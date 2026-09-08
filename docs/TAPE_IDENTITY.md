# Tape identity (content hash)

Spec Chum resolves a **display title** for inserted TAP/TZX files from a
content hash, falling back to the filesystem basename when unknown.

Tracked in [#366](https://github.com/mward-sudo/spec_chum/issues/366).

## Hash algorithm

**SHA-512 of the raw file bytes** (entire file on disk; no header stripping or
payload normalisation). Digests are **lowercase hex** (128 characters).

This matches the [ZXInfo API v3](https://api.zxinfo.dk/v3/) contract:

```http
GET /filecheck/{hash}
```

where `{hash}` is MD5 (32 hex chars) or SHA-512 (128 hex chars). Spec Chum
uses SHA-512 only.

## Lookup source (v1)

| Layer | Role |
| --- | --- |
| **Embedded local catalogue** | Metadata-only `hash → title` table in `formats::media_identity` (offline, no network). Ships titles for redistributable test fixtures only — **never** copyrighted tape images. |
| **Filename fallback** | On miss / I/O error: `path.file_name()` (same idea as macOS `lastPathComponent`). |
| **ZXInfo (optional / future)** | Live `GET https://api.zxinfo.dk/v3/filecheck/{sha512}` can enrich unknown files; **not required** for core emulation or UI. v1 does not call the network. |

Core emulation never depends on network or catalogue hits.

## Host surfaces

| Surface | Behaviour |
| --- | --- |
| **Shared** | `HostSession::media_title` / `media_sha512` set on tape open; cleared on eject |
| **Agent** | `GET /v1/status` includes optional `media_title` and `media_sha512` |
| **FFI** | `sc_media_title` / `sc_media_sha512` (heap strings; free with `sc_string_free`) |
| **egui** | Status text + top-bar tape label use the resolved title |
| **SpecChumMac** | Status footer / window media segment use `sc_media_title` after open |

## Extending the catalogue

Add `(sha512_hex, "Human Title")` rows to `LOCAL_CATALOGUE` in
`crates/formats/src/media_identity.rs`. Prefer redistributable / fixture media
only. To compute a digest:

```bash
shasum -a 512 path/to/file.tap
```

## Non-goals (v1)

- Shipping copyrighted tape images
- Perfect TOSEC renaming of a user’s library
- Network-required title resolution
