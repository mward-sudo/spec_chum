// Embed Windows PE icon for release `spec_chum.exe` (Refs #231).
//
// Only runs when targeting Windows. Soft-skips when the host lacks RC tooling
// (local cross-compile); CI Windows runners must succeed.

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }

    let mut res = winres::WindowsResource::new();
    res.set_icon("assets/icon.ico");
    match res.compile() {
        Ok(()) => {}
        Err(err) if std::env::var_os("CI").is_some() => {
            panic!("winres failed to embed app icon in CI: {err}");
        }
        Err(err) => {
            println!("cargo:warning=winres skipped (no Windows RC tooling?): {err}");
        }
    }
}
