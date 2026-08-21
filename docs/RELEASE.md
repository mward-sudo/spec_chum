# Cutting a Spec Chum release

GitHub Actions builds **macOS**, **Linux**, and **Windows** binaries and attaches
them to a GitHub Release when a version tag is pushed.

The product binary is the egui host `spec_chum` (plus `spec-chum-debug`).
System ROMs are never packaged. Native SwiftUI `.app` packaging is tracked
separately ([#68](https://github.com/mward-sudo/spec_chum/issues/68)).

## Before tagging (required)

Do **not** push a `vX.Y.Z` tag (and do not treat merge-then-tag as done) until
the **full slow test suite** passes on the commit you intend to release.
Default PR CI and `./scripts/check.sh` are not enough.

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

2. **system-tests** — third-party ULA/ROM TAP suite (`--features system-tests`;
   fixtures fetched into `.rom-cache/system-tests/`):

   ```bash
   ./scripts/run_system_tests.sh
   ```

3. **z80full** — full Patrik Rak CPU suite (`#[ignore]` day-to-day / not default
   CI; **required for release**). Included when you use `run_slow_tests.sh`, or:

   ```bash
   SYSTEM_TESTS_Z80FULL=1 ./scripts/run_system_tests.sh
   # equivalent:
   ./scripts/fetch_z80test.sh
   cargo test -p machine --features slow-tests --release z80full -- --nocapture --ignored
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
  (`z80doc.tap` is in git; `z80full.tap` is extracted there when fetched)

Failures are real accuracy misses — do not stub them to ship.

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
git tag -a v0.1.0 -m "Spec Chum 0.1.0"
git push origin v0.1.0
```

5. The [Release](../.github/workflows/release.yml) workflow runs on `v*.*.*`
   tags. It also supports **Actions → Release → Run workflow** with an existing
   tag if you need to rebuild assets. The workflow builds and publishes
   binaries; it does **not** re-run the slow suite — maintainers/agents must
   have already passed `./scripts/run_slow_tests.sh` before tagging.

Archives look like:

```text
spec-chum-0.1.0-x86_64-unknown-linux-gnu.tar.gz
spec-chum-0.1.0-x86_64-pc-windows-msvc.zip
spec-chum-0.1.0-aarch64-apple-darwin.tar.gz
spec-chum-0.1.0-x86_64-apple-darwin.tar.gz
SHA256SUMS
SHA256SUMS.asc          # only if GPG_PRIVATE_KEY is set
```

Each archive contains `spec_chum` / `spec_chum.exe`, `spec-chum-debug` /
`spec-chum-debug.exe`, `LICENSE`, and a short `README.txt` (no `roms/`).

Linux hosts need GTK 3 and ALSA runtime libraries (`libgtk-3-0` and
`libasound2` on Debian/Ubuntu).

A `workflow_dispatch` rebuild must pass an existing `vX.Y.Z` tag to publish;
an empty tag still builds `dev-<sha>` artifacts only.

## Signing (optional)

Unsigned assets still publish. Signing steps **no-op** when the matching secret
is absent so a first release does not require certificates.

| Platform | What | Repository secrets |
| --- | --- | --- |
| All | SHA-256 checksums | none |
| All | [GitHub Artifact Attestations](https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations) (Sigstore) | none (OIDC) |
| macOS | Developer ID `codesign` (hardened runtime + timestamp) | `APPLE_CERTIFICATE_P12_BASE64`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` |
| Windows | Authenticode (`signtool`, SHA-256, DigiCert timestamp) | `WINDOWS_PFX_BASE64`, `WINDOWS_PFX_PASSWORD` |
| Checksums | Detached ASCII-armored GPG signature `SHA256SUMS.asc` | `GPG_PRIVATE_KEY`, optional `GPG_PASSPHRASE` |

`APPLE_CERTIFICATE_P12_BASE64` / `WINDOWS_PFX_BASE64` are base64 of the `.p12` /
`.pfx` file (`base64 -i cert.p12 | pbcopy`). Identity is the Common Name of the
Developer ID Application certificate, for example
`Developer ID Application: Example Ltd (TEAMID)`.

Notarizing a full `.app` / DMG is not part of this workflow (CLI tools only).
That packaging is tracked in [#68](https://github.com/mward-sudo/spec_chum/issues/68).

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
gh attestation verify spec-chum-0.1.0-aarch64-apple-darwin.tar.gz \
  --owner mward-sudo --repo spec_chum
```
