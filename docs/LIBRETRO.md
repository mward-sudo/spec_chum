# RetroArch / libretro core

`spec_chum_libretro` is an optional libretro core built on the shared
`host_api`. The normal Spec Chum applications do not depend on it.

## Build

```sh
cargo build --release -p spec_chum_libretro
```

The core library is written to Cargo's target directory as
`libspec_chum_libretro.dylib` (macOS), `libspec_chum_libretro.so` (Linux), or
`spec_chum_libretro.dll` (Windows). Load that file in RetroArch through
**Load Core → Install or Load a Core**.

Spec Chum system ROMs are not bundled with the core. Fetch them into the
repository's ignored `roms/` directory with `./scripts/fetch_roms.sh` before
launching content.

## Content and controls

The core loads `.tap`, `.tzx`, `.sna`, `.z80`, `.rzx`, `.dsk`, and `.trd`
content by path. It selects +3 for DSK, 128K for TRD, and 48K for other formats;
snapshot loading may select the model encoded by the snapshot. Tape content
starts with the standard `LOAD ""` command and playback enabled.

The frontend's keyboard input maps letters, digits, Enter, Space, Shift, Ctrl,
and Alt to the Spectrum keyboard matrix. Arrow keys and Tab also control the
Kempston joystick; joypad directions and A map to Kempston directions and fire.
Video uses XRGB8888-compatible pixels at 50 Hz, with mono emulator audio sent
to both output channels at the host sample rate.

The core requires full-path content access because the shared media loaders
take file paths. A RetroArch executable is not required to build the workspace,
but loading content in RetroArch is the frontend integration check.
