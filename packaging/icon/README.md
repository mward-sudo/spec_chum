# Spec Chum app icon

Shared mark for packaged hosts ([#231](https://github.com/mward-sudo/spec_chum/issues/231)):

| Asset | Used by |
| --- | --- |
| `spec-chum-1024.png` (and 256/512) | Source masters |
| `../linux/spec-chum.png` | AppImage, `.deb`, `.desktop` (`linux_shell` release primary) |
| `../macos/AppIcon.icns` | egui `Spec Chum.app` / DMG |
| `../windows/spec-chum.ico` | Inno Setup wizard + Start Menu; `windows_shell` PE (`winres`; release primary) |
| `../../crates/app/assets/icon.png` | egui window icon |
| `../../crates/app/assets/icon.ico` | egui Windows PE resource (`winres`) |

The source master is the supplied Spectrum Enter artwork in
`spec-chum-1024.png`. The supplied Windows `.ico`, macOS `.icns` and iconset,
and Linux PNG are retained here so regeneration does not depend on the original
download. The generator resizes the master for egui and size-specific PNGs.

Regenerate (Pillow; macOS `iconutil` rebuilds `.icns` from the retained iconset):

```bash
python3 scripts/generate_app_icons.py
```

Keep the Spectrum Enter key and rainbow artwork consistent across app surfaces.
