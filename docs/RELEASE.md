# Cutting a Spec Chum release

GitHub Actions builds **macOS**, **Linux**, and **Windows** archives and attaches
them to a GitHub Release when a version tag is pushed.

The product binary is the egui host `spec_chum`. Headless debugger and agent HTTP
live on the **same** binary (`spec_chum --serve`, `spec_chum debug …`). System ROMs
are never packaged.

On macOS, release CI wraps the egui binary in a production **`Spec Chum.app`**
bundle and ships a **`.dmg`** (primary) with an **Applications** folder shortcut
plus a secondary `.zip` of the same tree. When Apple notary secrets are set,
CI notarises and staples the `.dmg` (and staples the staged `.app` for the
secondary zip) — see signing table below ([#354](https://github.com/mward-sudo/spec_chum/issues/354),
Refs [#231](https://github.com/mward-sudo/spec_chum/issues/231)). Windows ships a
**portable `.zip`** and an **Inno Setup `*-setup.exe`** (Start Menu + uninstall).
Linux AppImage/`.deb` and a shared app icon remain on #231. Native UI shells are
separate ([#351](https://github.com/mward-sudo/spec_chum/issues/351)).

## Before tagging (required)

Do **not** push a `vX.Y.Z` tag (and do not treat merge-then-tag as done) until
the **full slow test suite** passes on the commit you intend to release.
Default PR CI and `./scripts/check.sh` are not enough. Tier overview:
[TESTING.md](TESTING.md).

### Full slow test suite

One command (preferred):

```bash
./scripts/run_slow_tests.sh
```

That is exactly:

1. **z80doc** — Patrik Rak z80test documented suite (`--features slow-tests`;
   fixture in `tests/fixtures/z80test/`):

   ```bash
   ./scripts/fetch_roms.sh
   cargo test -p machine --features slow-tests --release z80doc_all_tests_passed -- --nocapture
   ```

2. **z80ccf** — SCF/CCF after every instruction (Q-sensitive Zilog behaviour):

   ```bash
   cargo test -p machine --features slow-tests --release z80ccf_all_tests_passed -- --nocapture
   ```

3. **z80memptr** — MEMPTR register via `BIT n,(HL)` after each instruction:

   ```bash
   cargo test -p machine --features slow-tests --release z80memptr_all_tests_passed -- --nocapture
   ```

4. **system-tests** — third-party ULA/ROM TAP suite (`--features system-tests`;
   fixtures fetched into `.rom-cache/system-tests/`):

   ```bash
   ./scripts/run_system_tests.sh
   ```

5. **z80full** — full Patrik Rak CPU suite under `--features slow-tests`
   (fixture checked in at `tests/fixtures/z80test/z80full.tap`; **required for
   release**). Included when you use `run_slow_tests.sh`, or:

   ```bash
   SYSTEM_TESTS_Z80FULL=1 ./scripts/run_system_tests.sh
   # equivalent:
   cargo test -p machine --features slow-tests --release z80full_all_tests_passed -- --nocapture
   ```

Also run the usual fast gate on that same commit:

```bash
./scripts/check.sh
```

Needs network once for ROMs, system TAPs, and (if missing) the z80test archive.
Distinguish cache vs fixture paths:

- ROM images: `roms/` (via `./scripts/fetch_roms.sh`)
- System-test TAPs: `.rom-cache/system-tests/`
- z80test release archive cache: `.rom-cache/z80test-1.2a.zip` (via
  `./scripts/fetch_z80test.sh`)
- Extracted CPU fixtures used by the suite: `tests/fixtures/z80test/`
  (`z80doc.tap` and `z80full.tap` are in git; `./scripts/fetch_z80test.sh` can
  refresh them from `.rom-cache/z80test-1.2a.zip` if needed)

Failures are real accuracy misses — do not stub them to ship.

## Version numbers (semver)

Workspace crates and GitHub Release tags ship together as `vX.Y.Z` (root
`[workspace.package] version` in `Cargo.toml`).

| Bump | When |
| --- | --- |
| **Patch** | Bug fixes, accuracy corrections, refactors, tuning that does **not** add new user- or agent-visible capability. |
| **Minor** | New user-facing or agent-visible capability since the last **published** tag — Agent Debug HTTP routes, host screenshot capture, new machine models, major living-room work, new tape/disk surfaces, etc. |
| **Major** | Breaking changes to supported platforms, default behaviour, or the Agent Debug API contract. |

Before tagging, compare against the previous **published** release (not an
intermediate mistaken tag). Keep workspace crate versions, macOS bundle numeric
version (derived from the tag in `stage-macos-egui-app.sh`), and archive names
aligned.

If a tag shipped at the wrong semver level: delete the GitHub Release and remote
tag (`gh release delete vX.Y.Z --yes`; `git push origin :refs/tags/vX.Y.Z`),
bump to the correct version, re-run the gates above, tag again, and note
supersession in the replacement release body (v0.4.2 → v0.5.0).

## Cut a release

1. Version is `[workspace.package] version` in the root `Cargo.toml` (currently
   inherited by every crate).
2. Commit any version bump on `main`.
3. Confirm `./scripts/check.sh` and `./scripts/run_slow_tests.sh` are green on
   that commit (see above).
4. Tag and push (annotated tags preferred):

```bash
git checkout main
git pull
git tag -a v0.2.0 -m "Spec Chum 0.2.0"
git push origin v0.2.0
```

5. The [Release](../.github/workflows/release.yml) workflow runs on `v*.*.*`
   tags. It also supports **Actions → Release → Run workflow** with an existing
   tag if you need to rebuild assets. The workflow builds and publishes
   binaries; it does **not** re-run the slow suite — maintainers/agents must
   have already passed `./scripts/run_slow_tests.sh` before tagging.

### Artifact layout

| Platform | Archive | Contents |
| --- | --- | --- |
| Linux | `spec-chum-<ver>-x86_64-unknown-linux-gnu.tar.gz` | `spec_chum`, `LICENSE`, `README.txt` |
| Windows (portable) | `spec-chum-<ver>-x86_64-pc-windows-msvc.zip` | `spec_chum.exe`, `LICENSE`, `README.txt` |
| Windows (installer) | `spec-chum-<ver>-x86_64-pc-windows-msvc-setup.exe` | Inno Setup: Start Menu + uninstall; installs `spec_chum.exe` + LICENSE/README |
| macOS (Apple silicon) | `spec-chum-<ver>-aarch64-apple-darwin.dmg` (**primary**) | `Spec Chum.app/`, `Applications` → `/Applications`, `LICENSE`, `README.txt` |
| macOS (Apple silicon) | `spec-chum-<ver>-aarch64-apple-darwin.zip` (secondary) | `Spec Chum.app/`, `LICENSE`, `README.txt` |
| macOS (Intel) | same pair with `x86_64-apple-darwin` | same layout |

One primary application per platform ([#231](https://github.com/mward-sudo/spec_chum/issues/231)).
Prefer the macOS **`.dmg`**: open it and drag **Spec Chum.app** onto **Applications**.
On Windows, prefer the **`*-setup.exe`** for Start Menu / uninstall, or the portable
`.zip` for unzip-and-run. Headless use:

```bash
spec_chum --serve --model 48k
spec_chum debug dump-state
spec_chum debug --tap path/to/game.tap type-load --code
```

(`spec-chum-debug` remains a source-build alias via `cargo run -p debug_cli`; it is
**not** attached to GitHub Release archives.)

Linux uses `.tar.gz` (common on Unix); Windows ships a portable `.zip` and an
Inno Setup `*-setup.exe`; macOS ships a primary `.dmg` and a secondary `.zip`.
No `roms/` are included.

Checksums and optional signatures:

```text
SHA256SUMS
SHA256SUMS.asc          # only if GPG_PRIVATE_KEY is set
```

Linux hosts need GTK 3, ALSA, and udev runtime libraries (`libgtk-3-0`,
`libasound2`, and `libudev1` on Debian/Ubuntu). Build images also need
`libudev-dev` / `pkg-config` for `gilrs`.

A `workflow_dispatch` rebuild must pass an existing `vX.Y.Z` tag to publish;
an empty tag still builds `dev-<sha>` artifacts only.

## Signing (optional)

Unsigned assets still publish. Signing steps **no-op** when the matching secret
is absent so a first release does not require certificates.

| Platform | What | Repository secrets |
| --- | --- | --- |
| All | SHA-256 checksums | none |
| All | [GitHub Artifact Attestations](https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations) (Sigstore) | none (OIDC) |
| macOS | Developer ID `codesign` on `Spec Chum.app` and the release `.dmg` (hardened runtime + timestamp on Mach-O / `.app`; DMG signed when the same secrets are set) | `APPLE_CERTIFICATE_P12_BASE64`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` |
| macOS | Notarisation + staple (`notarytool submit --wait`, then `stapler staple` on the `.dmg` and staged `.app`) | **Preferred:** `APPLE_API_KEY_P8_BASE64`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID`. **Fallback:** `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, `APPLE_TEAM_ID` |
| Windows | Authenticode (`signtool`, SHA-256, DigiCert timestamp) on `spec_chum.exe` and the Inno `*-setup.exe` | `WINDOWS_PFX_BASE64`, `WINDOWS_PFX_PASSWORD` |
| Checksums | Detached ASCII-armored GPG signature `SHA256SUMS.asc` | `GPG_PRIVATE_KEY`, optional `GPG_PASSPHRASE` |

`APPLE_CERTIFICATE_P12_BASE64` / `WINDOWS_PFX_BASE64` are base64 of the `.p12` /
`.pfx` file (`base64 -i cert.p12 | pbcopy`). Identity is the Common Name of the
Developer ID Application certificate, for example
`Developer ID Application: Example Ltd (TEAMID)`.

**Notarisation** ([#354](https://github.com/mward-sudo/spec_chum/issues/354)):
after the `.dmg` is codesigned, `scripts/ci/notarize-macos.sh` submits it with
`xcrun notarytool`, waits for Accepted, staples the `.dmg`, then staples the
staged `Spec Chum.app` so the secondary `.zip` is also offline-friendly. Prefer
an **App Store Connect API key** (Team key: Issuer UUID + Key ID + `.p8`
base64). Apple ID + app-specific password + Team ID works as a fallback. When
neither credential set is complete, the step **no-ops** (codesigned-but-
unnotarised assets still publish; Gatekeeper may warn until secrets are set).
Notary secrets are independent of the Developer ID `.p12` — you need both for a
fully Gatekeeper-clean release.

Verify a notarised download (after install / mount):

```bash
spctl --assess --type open --verbose=4 /path/to/Spec\ Chum.app
xcrun stapler validate /path/to/spec-chum-….dmg
```

Default PR CI (`.github/workflows/ci.yml`) is unchanged and does not use these
secrets. A `workflow_dispatch` without a tag still builds archives as
`dev-<sha>` artifacts; it does not create a GitHub Release.

## Verify a download

```bash
sha256sum -c SHA256SUMS
# if SHA256SUMS.asc is present:
gpg --verify SHA256SUMS.asc SHA256SUMS
```

Attestations from the release workflow run:

```bash
gh attestation verify spec-chum-0.2.0-aarch64-apple-darwin.dmg \
  --owner mward-sudo --repo spec_chum
# secondary zip still attested when published:
gh attestation verify spec-chum-0.2.0-aarch64-apple-darwin.zip \
  --owner mward-sudo --repo spec_chum
```