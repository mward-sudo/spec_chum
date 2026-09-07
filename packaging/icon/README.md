# Spec Chum app icon

Shared mark for packaged hosts ([#231](https://github.com/mward-sudo/spec_chum/issues/231)):

| Asset | Used by |
| --- | --- |
| `spec-chum-1024.png` (and 256/512) | Source masters |
| `../linux/spec-chum.png` | AppImage, `.deb`, `.desktop` |
| `../macos/AppIcon.icns` | egui `Spec Chum.app` / DMG |
| `../windows/spec-chum.ico` | Inno Setup wizard + Start Menu |
| `../../crates/app/assets/icon.png` | egui window icon |
| `../../crates/app/assets/icon.ico` | Windows PE resource (`winres`) |

Regenerate (Pillow + macOS `iconutil` for `.icns`):

```bash
python3 scripts/generate_app_icons.py
```

Design: dark CRT bezel, green BASIC block cursor, classic Spectrum rainbow stripe.
Do not replace with generic gradient / “AI purple” art.
