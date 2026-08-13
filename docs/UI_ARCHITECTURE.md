# UI architecture

How Spec Chum should present the emulator to users, and why we keep the current stack for now.

## Recommendation (default)

**Keep Rust core + egui/eframe**, polish carefully, and avoid fake “liquid glass” / fullsize content under the macOS titlebar.

Spec Chum’s scarce resource is **hardware accuracy** (Z80, ULA, tape), not UI novelty. egui already ships framebuffer blit, menus, file dialogs (`rfd`), and headless `Context::run` tests. A multi-shell rewrite would burn months without improving T-state fidelity.

Revisit native shells only if we hit a hard wall on platform menus, accessibility, or App Store packaging that egui cannot meet.

## Comparison (emulator frontends)

| Option | Framebuffer | Audio | Input latency | Menus / dialogs | macOS polish | Fit for Spec Chum |
| --- | --- | --- | --- | --- | --- | --- |
| **egui / eframe** (current) | Texture upload / `ColorImage` — good | Via `cpal` beside egui — fine | Immediate-mode; low if we avoid heavy UI work per frame | Custom menus + `rfd`; not native NSMenu | “OK” if we stay below the titlebar; fake glass under traffic lights was a mistake | **Best default** |
| **iced** | Solid wgpu path | Same `cpal` pattern | Elm-style; good if messages stay thin | Custom widgets; dialogs via crates | Cleaner declarative layout; still not AppKit | Strong alt if we outgrow immediate mode; migration cost high |
| **dioxus-desktop** | WebView or custom; blit often via canvas/WebGL | WebAudio or native bridge | Extra hop through web stack | HTML/CSS familiar; file dialogs possible | Can look native-ish with CSS; not true Cocoa | Weak for cycle-accurate emu (latency, GPU path complexity) |
| **Slint** | Designed for embedded UIs; can host images | External | Good | Declarative; tooling strong | Polished widgets; still not NSMenu | Possible later; DSL + tooling tax for a small team |
| **Tauri + web** | Canvas/WebGL blit | WebAudio or plugin | IPC + browser — usually worse | Excellent HTML/CSS menus; native dialogs | Can look great with CSS | Overkill; process split hurts hot-path simplicity |
| **gpui** (Zed) | GPU-first, powerful | Possible but immature ecosystem for apps like ours | Excellent in editor context | Custom | Very Mac-capable in Zed’s hands | High churn / API risk; not aimed at emulators |
| **Rust core + native shells** (SwiftUI / WinUI / GTK) | Best-in-class per OS if we invest | Best-in-class per OS | Best if wired carefully | True native menus | Best polish | Ideal “product” endgame; **3× UI maintenance** for a small team |

### Tradeoffs that matter here

1. **Framebuffer blit** — We need ~50 Hz 352×296 (or similar) RGBA uploads. egui textures and iced/wgpu are both fine. WebView paths add copies and vsync quirks.
2. **Audio** — Beeper/AY already run through `cpal`. UI crate choice should not own the audio thread; keep that separation.
3. **Input latency** — Matrix keyboard + Kempston want raw key edges each frame. Immediate-mode egui is fine; avoid web IPC for keys.
4. **Menus / file dialogs** — Native NSMenu would fix “feel,” but egui menus work if hit zones are real (normal titlebar, opaque panels). `rfd` covers Open dialogs.
5. **macOS polish** — Fullsize content view + translucent panels looked clever and broke clicking (#60). Prefer boring, opaque chrome under a normal titlebar.

## Decision record

| Choice | Rationale |
| --- | --- |
| Stay on **egui/eframe** | Already integrated; testable headless; matches small-team bandwidth |
| No fullsize content under titlebar | Hit-testing and traffic lights must stay out of the menu strip |
| Opaque panels | Predictable contrast and clicks; drop fake glass |
| Native multi-shell later (optional) | Only if packaging/a11y/menus become product-critical |
| Avoid Tauri/Dioxus-web for the machine loop | Extra process/IPC is the wrong complexity for a Spectrum core |

## Practical chrome rules (current shell)

- Normal window titlebar (do not draw the menu under traffic lights).
- Opaque top menu strip with a minimum clickable height.
- Status text for tape play/pause; Play must arm EAR + flash-load (`playing == true`).
- Prefer `rfd` for file picks; keep ROM/tape/disk logic in `machine` / `tape` / `formats`.
