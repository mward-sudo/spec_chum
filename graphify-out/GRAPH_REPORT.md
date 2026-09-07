# Graph Report - uc2i  (2026-09-07)

## Corpus Check
- 184 files · ~225,258 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 4267 nodes · 11569 edges · 185 communities (154 shown, 25 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 443 edges (avg confidence: 0.84)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `c823bcc2`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- ApiError
- host_api/src/ffi.rs
- Plus3Fdc
- trace/src/lib.rs
- rom.rs
- HostBridge
- AgentClient
- HostError
- tape/src/lib.rs
- Interface1
- ula/src/lib.rs
- system_tests.rs
- .new_48k
- ControlPlane
- BetaDisk
- tzx.rs
- DivMmc
- SpectrumNSView
- host_api/src/keymap.rs
- Machine
- apply_zoom_camera
- src/session.rs
- living_room/src/ffi.rs
- hybrid_state_machine
- .cpu_mut
- ContentView
- image_copy.rs
- BusPlus3
- .livingRoomToolbar
- TapeAudioPlayer
- machine_config.rs
- .host_mut
- Model
- video.rs
- .step_once
- LivingRoomNSView
- .recompose_input
- formats/src/lib.rs
- prefs.rs
- PrefModelSlug
- SpecChumApp
- HostViewState
- opcodes.rs
- RomSetupSlot
- machine/src/lib.rs
- Multiface1
- service.rs
- CodingKeys
- parse_model
- snow.rs
- bus/src/lib.rs
- custom_loader_matrix_models_instant_and_ear
- ui_overlay.rs
- .new
- HeadlessRoom
- Debugger
- ModelId
- .onLivingRoomDisplayTick
- TimexScld
- EmulatorHost
- Ay8912
- FormatError
- glow.rs
- PresentTarget
- Cpu
- FramebufferMeta
- Ula48
- camera.rs
- fuse.rs
- PrefModel
- Bus48
- dck.rs
- .new
- FlatMem
- .with_session_ref
- Vec
- joystick.rs
- auth_empty
- tick_emulator
- aperture_debug_enabled
- headless.rs
- Self
- control_plane/src/present.rs
- setup_room
- crt.rs
- router
- .syncMatrix
- upload_external_framebuffer
- Keyboard
- TimexDock
- cpu.rs
- flags.rs
- Agent Debug HTTP API
- disasm.rs
- KempstonMouse
- rzx.rs
- mod.rs
- agent_embed.rs
- spawn
- RoomPerf
- living_room/src/keymap.rs
- quality.rs
- machine
- machine crate
- AppKit
- AppState
- .with_host_mut
- attach_crt_to_television
- media.rs
- host_api crate
- Full slow test suite
- living_room/src/lib.rs
- import_iosurface_texture
- routes/session.rs
- Kempston
- control_plane/src/window_capture.rs
- OpenMediaDialog
- check_pr_reviews.sh
- Spec Chum
- PrefsLock
- theme.rs
- room_perf.rs
- Snapshot128
- custom_loader_tap
- CodeRabbit merge gate
- inspect.rs
- fb_scale.rs
- fetch_roms.sh
- app crate
- Flash-load at LD-BYTES 0x056C
- PrefAyStereo
- fetch_system_tests.sh
- room_perf_matrix.sh
- trace_dump
- UiPreferences
- CrtPlugin
- HybridPlugin
- Test tier matrix
- .inspect
- UiOverlayPlugin
- check_crates.sh
- sign-macos.sh
- graphify_install_hooks.sh
- build_macos_app.sh
- check.sh
- check_living_room.sh
- stage-macos-egui-app.sh
- fetch_living_room_assets.sh
- make_spectrum_dck.sh
- trace crate
- Package.swift
- ./scripts/check_living_room.sh
- ./scripts/build_macos_app.sh
- graphify_update.sh
- fetch_z80test.sh
- run_macos_app.sh
- run_slow_tests.sh
- run_system_tests.sh
- stage_living_room_assets.sh
- .from_model
- ./scripts/check_crates.sh
- gh stack workflow
- HostSession
- .setFlashLoad
- RomLoadError
- health
- audio.rs
- .load_rom_bytes_with_overrides
- .body
- is_screen_tri
- .open_tape
- MachineConfigEditorView
- NSEvent
- crt_phosphor_local
- input.rs
- .share_host
- .installTrackingArea
- RoomPlugin
- check_deny.sh

## God Nodes (most connected - your core abstractions)
1. `HostBridge` - 215 edges
2. `HostSession` - 134 edges
3. `Machine` - 125 edges
4. `ControlPlane` - 106 edges
5. `AppState` - 77 edges
6. `session_mut()` - 66 edges
7. `HostError` - 65 edges
8. `Cpu` - 63 edges
9. `LivingRoomNSView` - 59 edges
10. `BetaDisk` - 55 edges

## Surprising Connections (you probably didn't know these)
- `formats crate` --references--> `synthetic_plus3_boot_marker`  [INFERRED]
  AGENTS.md → tests/fixtures/plus3/README.md
- `ula crate` --references--> `ulatest3.tap`  [INFERRED]
  AGENTS.md → tests/fixtures/system/README.md
- `z80 crate` --references--> `Fuse Z80 test vectors`  [EXTRACTED]
  AGENTS.md → tests/fixtures/fuse/README.md
- `Patrik Rak z80test` --references--> `Spec Chum`  [EXTRACTED]
  tests/fixtures/z80test/README.md → README.md
- `host_api crate` --references--> `SpecChumMac SwiftUI shell`  [EXTRACTED]
  AGENTS.md → docs/MACOS_NATIVE.md

## Import Cycles
- 1-file cycle: `crates/living_room/src/present_metal.rs -> crates/living_room/src/present_metal.rs`
- 2-file cycle: `crates/z80/src/cpu.rs -> crates/z80/src/opcodes.rs -> crates/z80/src/cpu.rs`
- 2-file cycle: `crates/bus/src/lib.rs -> crates/bus/src/multiface.rs -> crates/bus/src/lib.rs`
- 2-file cycle: `crates/bus/src/beta_disk.rs -> crates/bus/src/lib.rs -> crates/bus/src/beta_disk.rs`
- 2-file cycle: `crates/bus/src/divmmc.rs -> crates/bus/src/lib.rs -> crates/bus/src/divmmc.rs`

## Communities (185 total, 25 thin omitted)

### Community 0 - "ApiError"
Cohesion: 0.36
Nodes (5): ApiError, ErrorBody, From, Self, String

### Community 1 - "host_api/src/ffi.rs"
Cohesion: 0.08
Nodes (107): break_reason_code(), clear_last_error(), ffi_bad_model_returns_null(), ffi_create_destroy_and_run(), ffi_debug_dump_json_and_peek_null(), ffi_joystick_mode_rejects_truncated_overflow(), ffi_mouse_delta_and_buttons_smoke(), heap_cstring() (+99 more)

### Community 2 - "Plus3Fdc"
Cohesion: 0.05
Nodes (55): cpm_dir_entry(), DskImage, find_id_matches_r_without_chrn_c(), multi_sector_dsk_lookup(), parse_and_read_sector(), parse_track(), plus3_basic_poke_marker(), plus3_cpm_chs() (+47 more)

### Community 3 - "trace/src/lib.rs"
Cohesion: 0.06
Nodes (59): AsRef, BitOr, BitOrAssign, BufWriter, append_flushes_events_to_trace_file(), AppendSink, categories(), Category (+51 more)

### Community 4 - "rom.rs"
Cohesion: 0.09
Nodes (80): expected_main_rom_bytes(), exrom_available(), exrom_available_in(), exrom_candidates(), install_rom_slot(), install_rom_slot_copies_to_expected_path(), install_rom_slot_validates_size(), main_rom_available() (+72 more)

### Community 5 - "HostBridge"
Cohesion: 0.04
Nodes (40): Float, JoystickMode, UInt32, String, HostBridge, .isCustomConfigActive, .joystickMode, .kempstonMouse (+32 more)

### Community 6 - "AgentClient"
Cohesion: 0.07
Nodes (35): Agent, AgentClient, AuthRequest, Option, Result, Self, String, Value (+27 more)

### Community 7 - "HostError"
Cohesion: 0.10
Nodes (7): HostError, peek_poke_and_inspect_json(), Error, Path, Result, String, BreakReason

### Community 8 - "tape/src/lib.rs"
Cohesion: 0.07
Nodes (44): game_running(), load_deathchase(), plus2a_48basic_instant_still_runs_deathchase(), plus2a_menu_loader_ear_runs_deathchase(), plus2a_menu_loader_instant_runs_deathchase(), rom_plus2a(), Machine, Option (+36 more)

### Community 9 - "Interface1"
Cohesion: 0.07
Nodes (26): decode_port(), Drive, If1Port, Interface1, Interface1RomError, mdr_roundtrip_via_if1(), motor_select_and_sector_stream_read(), motor_select_and_sector_stream_write() (+18 more)

### Community 10 - "ula/src/lib.rs"
Cohesion: 0.12
Nodes (13): contention_delay(), contention_delay_128(), contention_delay_48(), contention_delay_params(), framebuffer_dims(), io_contention_extra(), io_contention_extra_128(), io_contention_extra_48() (+5 more)

### Community 11 - "system_tests.rs"
Cohesion: 0.09
Nodes (70): amstrad_plus2a_no_snow_with_i_40(), amstrad_plus3_no_snow_with_i_40(), assert_contended_nop_pattern(), assert_no_snow_with_i_40(), assert_pattern(), assert_screen_has(), assert_snow_disrupts_testcard(), azesmbog_loads_and_paints() (+62 more)

### Community 12 - ".new_48k"
Cohesion: 0.15
Nodes (38): apply_sna48_sets_pc_ram_and_border(), attr_mark_code_ok(), attr_mark_ear_load_quotes_code_succeeds_at_speed_10(), attr_mark_experience_load_succeeds(), attr_mark_fixture_flash_loads_code_bytes(), attr_mark_load_path_dumps_trace_on_failure(), attr_mark_load_path_must_succeed(), boggit_header_flash_loads_when_present() (+30 more)

### Community 13 - "ControlPlane"
Cohesion: 0.09
Nodes (6): ControlPlane, ApiResult, Mutex, Path, SharedHostView, UserMachineConfig

### Community 14 - "BetaDisk"
Cohesion: 0.06
Nodes (60): beta_fdc_emits_disk_trace(), BetaDisk, BetaDiskPatchError, disk_not_ready_without_image(), drive_b_is_not_ready(), feed_write_track_sector(), force_interrupt_clears_drq(), m1_48k_pages_at_3c00_128k_does_not() (+52 more)

### Community 15 - "tzx.rs"
Cohesion: 0.10
Nodes (30): active_pulse_counters_are_block_relative(), advance_clears_high_ear_on_exact_final_pulse_end(), advance_clears_playing_on_exact_final_pulse_boundary(), append_pure_data(), append_standard_block(), append_turbo_block(), empty_tzx_reports_zero_blocks(), minimal_tzx_standard() (+22 more)

### Community 16 - "DivMmc"
Cohesion: 0.11
Nodes (22): automap_entry_and_exit_with_eeprom(), automap_ignored_without_eeprom_or_mapram(), control_port_conmem_shows_ram_page(), DivMmc, eeprom_accepts_larger_image_prefix(), mapram_is_sticky_across_control_writes(), mapram_uses_page_3_in_lower_8k(), Default (+14 more)

### Community 17 - "SpectrumNSView"
Cohesion: 0.09
Nodes (16): SpectrumNSView, .acceptsFirstResponder, .canBecomeKeyView, .focusRingMaskBounds, .isFlipped, Any, Bool, Context (+8 more)

### Community 18 - "host_api/src/keymap.rs"
Cohesion: 0.09
Nodes (36): chord_for(), letter_digit(), modifier_keys(), mods_shift(), punct_chord(), quote_alone_is_symbol_7(), quote_shift_is_symbol_p(), Key (+28 more)

### Community 19 - "Machine"
Cohesion: 0.06
Nodes (17): custom_loader_ok(), DivMmcError, InsertDiskError, interface1_opcode_fetch_pages_shadow_rom(), interface1_rom_load_skips_cleanly_when_missing(), Interface1Error, Machine, Model (+9 more)

### Community 20 - "apply_zoom_camera"
Cohesion: 0.09
Nodes (37): Bloom, anim_eases_toward_target(), apply_zoom_camera(), CameraZoom, CrtLookBlend, IntroSkipRequest, nudge_clamps(), pose_at_zoom() (+29 more)

### Community 21 - "src/session.rs"
Cohesion: 0.16
Nodes (33): attach_beta_on_48k_and_reject_plus3(), border_toggle_resizes_framebuffer(), cursor_left_via_joystick_applies_caps_five(), dims(), joystick_kempston_mask_reaches_port(), kempston_arrow_left_does_not_pollute_matrix(), kempston_mouse_ports_after_synthetic_deltas(), load_dsk_rejects_non_plus3() (+25 more)

### Community 22 - "living_room/src/ffi.rs"
Cohesion: 0.18
Nodes (36): catch_const_u8(), catch_int(), catch_ptr(), catch_uint(), clear_last_error(), room_mut(), RoomHandle, c_char (+28 more)

### Community 23 - "hybrid_state_machine"
Cohesion: 0.16
Nodes (41): Camera, Children, LivingRoomCamera, apply_hybrid_display(), bind_present_target(), ensure_plate_image(), hybrid_state_machine(), HybridBakeCamera (+33 more)

### Community 24 - ".cpu_mut"
Cohesion: 0.14
Nodes (38): apply_trdos_find_boot_native_abi(), apply_trdos_run_native_abi(), beta_reads_synthetic_boot_basic_into_ram(), beta_trdos_rom_loop_reads_trd_sector_into_ram(), beta_write_track_via_synthetic_rom(), BetaDiskError, debug_trdos_line_new_handoff(), debug_trdos_run_pc_trace() (+30 more)

### Community 25 - "ContentView"
Cohesion: 0.08
Nodes (26): activateSpecChum(), AppDelegate, Notification.Name, ContentView, .body, .flatSpectrumChrome, .livingRoomChrome, .statusFooter (+18 more)

### Community 26 - "image_copy.rs"
Cohesion: 0.09
Nodes (36): Buffer, despawn_image_copiers(), drain_copied_frames(), image_copy_driver(), image_copy_extract(), ImageCopier, ImageCopiers, ImageCopyPlugin (+28 more)

### Community 27 - "BusPlus3"
Cohesion: 0.11
Nodes (17): BusPlus3, contended_banks_are_4_through_7(), fdc_motor_bit_on_1ffd_affects_st3(), fdc_read_data_protocol_via_ports(), is_contended_bank_plus3(), lock_blocks_both_ports(), no_floating_bus(), out_7ffd_address_does_not_hit_1ffd() (+9 more)

### Community 28 - ".livingRoomToolbar"
Cohesion: 0.08
Nodes (10): .livingRoomToolbar, Bool, Model, String, URL, UserMachineConfig, .experienceLoad, .instantLoad (+2 more)

### Community 29 - "TapeAudioPlayer"
Cohesion: 0.13
Nodes (21): AudioCaptureFile, AudioLog, Stats, Bool, Double, Float, Int, String (+13 more)

### Community 30 - "machine_config.rs"
Cohesion: 0.13
Nodes (27): AppliedConfig, apply_builtin_rom_when_no_override(), apply_diagrom_succeeds_on_128k_class_models(), apply_diagrom_succeeds_on_16k_class_models(), apply_diagrom_succeeds_on_plus3_with_16k_rom(), apply_rejects_bad_custom_rom_size(), apply_user_config(), build_machine() (+19 more)

### Community 32 - "Model"
Cohesion: 0.10
Nodes (20): Model, .id, pentagon128, .prefSlug, .requiresUserProvidedRoms, .romAvailable, .shortTitle, spectrum128 (+12 more)

### Community 33 - "video.rs"
Cohesion: 0.27
Nodes (17): default_png(), framebuffer(), FramebufferQuery, host_display(), host_window(), HostDisplayQuery, insert_meta_headers(), insert_present_headers() (+9 more)

### Community 34 - ".step_once"
Cohesion: 0.08
Nodes (27): ay_frame_audio_nonzero_when_tone_programmed(), emit_contend_sampled(), manual_read_track1_sector1(), mem_port_watch(), MemIo128, MemIo48, MemIoPlus3, next_frame_n() (+19 more)

### Community 35 - "LivingRoomNSView"
Cohesion: 0.09
Nodes (19): LivingRoomDisplayView, LivingRoomNSView, .acceptsFirstResponder, .canBecomeKeyView, .focusRingMaskBounds, .host, .isFlipped, Any (+11 more)

### Community 36 - ".recompose_input"
Cohesion: 0.14
Nodes (4): JoystickMode, Model, Self, UserMachineConfig

### Community 37 - "formats/src/lib.rs"
Cohesion: 0.18
Nodes (22): decode_z80_v1(), parse_sna128_regs_banks_pc(), parse_sna128_when_paged_is_bank5(), parse_z80_rejects_undersized_extended_header(), parse_z80_v1_compressed_regs_and_ram(), parse_z80_v1_uncompressed_regs_and_ram(), parse_z80_v2_128_banks_and_7ffd(), parse_z80_v2_pages_land_at_48k_addresses() (+14 more)

### Community 38 - "prefs.rs"
Cohesion: 0.14
Nodes (19): corrupt_file_falls_back_to_defaults(), custom_configs_round_trip(), load_prefs(), load_prefs_unlocked(), missing_file_falls_back_to_defaults(), model_rom_paths_round_trip(), recent_files_most_recent_first_deduped(), round_trip_preserves_fields() (+11 more)

### Community 39 - "PrefModelSlug"
Cohesion: 0.06
Nodes (38): JoystickMode, cursor, .id, kempston, sinclairLeft, sinclairRight, .title, HardwareCompatFlags (+30 more)

### Community 40 - "SpecChumApp"
Cohesion: 0.08
Nodes (23): BuildMachineError, App, BTreeMap, Context, Debug, Formatter, Frame, Instant (+15 more)

### Community 41 - "HostViewState"
Cohesion: 0.12
Nodes (14): HostViewState, HostWindowCapture, new_shared_host_view(), ApiResult, Arc, Debug, Formatter, Option (+6 more)

### Community 42 - "opcodes.rs"
Cohesion: 0.18
Nodes (31): add16(), adc16(), block_cp(), block_in(), block_ld(), block_out(), condition(), daa() (+23 more)

### Community 43 - "RomSetupSlot"
Cohesion: 0.26
Nodes (13): JsonRoot, JsonSlot, RomSetupCodec, RomSetupPayload, RomSetupSlot, .sizeHint, .statusColor, .statusLabel (+5 more)

### Community 44 - "machine/src/lib.rs"
Cohesion: 0.07
Nodes (40): advance_frame_t(), apply_snapshot128_plus3_applies_1ffd(), apply_snapshot128_z80_pages_and_7ffd(), apply_z80_snapshot48_sets_pc_ram_and_border(), attr_mark_type_load_128k_flash(), attr_mark_type_load_plus3_flash(), custom_loader_tap(), FrameAudio (+32 more)

### Community 45 - "Multiface1"
Cohesion: 0.12
Nodes (14): button_pages_on_nmi_vector(), in_9f_pages_in_in_1f_pages_out(), load_rom_size_check(), Multiface1, multiface1_port_match(), out_1f_clears_nmi_pending_without_unpaging(), out_3f_is_not_mf1_decode(), reset_clears_paging_keeps_ram() (+6 more)

### Community 46 - "service.rs"
Cohesion: 0.12
Nodes (22): apply_prefs_to_session(), capture_framebuffer_border_override_restores(), capture_framebuffer_restores_border_when_run_fails(), continue_and_eject_require_machine(), health_and_inspect_after_rom_load(), last_error_records_failures(), map_host_model_error(), mouse_requires_kempston_pref_then_accepts_input() (+14 more)

### Community 47 - "CodingKeys"
Cohesion: 0.12
Nodes (17): CodingKeys, attachBeta, attachDivmmc, attachInterface1, attachMultiface, ayStereo, base, customRomPath (+9 more)

### Community 48 - "parse_model"
Cohesion: 0.38
Nodes (6): Cli, main(), parse_model(), Option, Result, String

### Community 49 - "snow.rs"
Cohesion: 0.12
Nodes (16): corrupt_128_uses_alternate_bank_source(), corrupt_row32_col0_r_zero_not_skipped(), corrupt_skipped_when_r_matches_addr_lo(), corrupt_uses_refresh_low_byte_not_display(), double_duplicates_previous_column(), i_pointed_bank_128(), pattern_at_phase(), Option (+8 more)

### Community 50 - "bus/src/lib.rs"
Cohesion: 0.11
Nodes (30): beta_ports_when_trdos_paged_via_bus48(), beta_trdos_rom_overlays_when_paged(), bus128_m1_pages_trdos_at_3d00_not_3c00(), contend_128_differs_from_48_at_paper_start(), divmmc_automap_via_notify_m1(), divmmc_conmem_overlays_via_bus48(), divmmc_control_beats_interface1_on_shared_e3(), divmmc_eeprom_fixture_automaps_when_present() (+22 more)

### Community 51 - "custom_loader_matrix_models_instant_and_ear"
Cohesion: 0.28
Nodes (11): TimexDockError, attr_mark_load_matrix_models_and_speeds(), boggit_side1_matrix_when_present(), custom_loader_matrix_models_instant_and_ear(), MachineBuildError, peripheral_attach_rejects_unsupported_models_with_typed_errors(), rom_timex_tc2048(), Result (+3 more)

### Community 52 - "ui_overlay.rs"
Cohesion: 0.13
Nodes (27): BackgroundColor, Changed, ChildSpawnerCommands, CameraIntro, chrome_button(), chrome_buttons(), ChromeAction, host_cmd_shortcuts() (+19 more)

### Community 53 - ".new"
Cohesion: 0.17
Nodes (20): arrow_left_maps_joystick_kempston_and_cursor_mode(), debug_window_smoke_headless(), egui_menu_smoke_without_window(), emulator_session_uses_host_session(), gui_and_control_plane_share_live_session(), load_snapshot48_switches_from_128k(), load_snapshot48_switches_from_plus3(), physical_num1_survives_sinclair_left_joystick_clear() (+12 more)

### Community 54 - "HeadlessRoom"
Cohesion: 0.10
Nodes (8): HeadlessRoom, HeadlessRoomError, c_void, Debug, Formatter, Result, Self, Write

### Community 55 - "Debugger"
Cohesion: 0.12
Nodes (8): Debugger, Cell, Default, Option, Self, Vec, Watch, WatchHook

### Community 56 - "ModelId"
Cohesion: 0.17
Nodes (25): canonical_persist_path(), install_model_rom(), model_requires_user_rom(), model_rom_available(), model_rom_paths_snapshot(), pentagon_rom_setup_has_user_slots(), persisted_path_wins_over_missing_workspace(), rom_setup_json() (+17 more)

### Community 57 - ".onLivingRoomDisplayTick"
Cohesion: 0.09
Nodes (13): .livingRoomMode, InputLatencyProbe, Int32, String, Bool, CGFloat, Int, String (+5 more)

### Community 58 - "TimexScld"
Cohesion: 0.12
Nodes (8): altmembank_and_chunk_bits(), port_f4_latches(), port_ff_read_returns_last_write(), Option, Self, screen_mode_and_int_disable_from_port_ff(), TimexScld, TimexScreenMode

### Community 59 - "EmulatorHost"
Cohesion: 0.12
Nodes (14): EmulatorHost, HostPlugin, model_label(), App, Debug, Duration, Formatter, PathBuf (+6 more)

### Community 60 - "Ay8912"
Cohesion: 0.14
Nodes (15): acb_vs_abc_pan_differs(), acb_vs_abc_swap_b_and_c_pans(), Ay8912, ay_channel_b_only(), envelope_level(), envelope_write_restarts(), mixer_mute_silence(), mono_stereo_matches_sample_mono() (+7 more)

### Community 61 - "FormatError"
Cohesion: 0.16
Nodes (15): FormatError, Display, Error, Formatter, Result, String, decode_z80_page(), load_z80_v2_pages_128() (+7 more)

### Community 62 - "glow.rs"
Cohesion: 0.11
Nodes (21): CrtPhosphor, CrtFillLight, FrameGlow, GlowDriven, GlowPlugin, IncandescentLamp, red_border_dominates_glow(), App (+13 more)

### Community 63 - "PresentTarget"
Cohesion: 0.09
Nodes (23): blit_to_present(), extract_present_target(), ExtractedPresent, PresentBlitPlugin, PresentTarget, App, Arc, Commands (+15 more)

### Community 64 - "Cpu"
Cohesion: 0.23
Nodes (5): Cpu, Option, Vec, I, M

### Community 65 - "FramebufferMeta"
Cohesion: 0.20
Nodes (7): encode_framebuffer_png(), FramebufferMeta, parse_model_slug(), ApiResult, Option, Self, Vec

### Community 66 - "Ula48"
Cohesion: 0.18
Nodes (6): bank_switch_between_bitmap_and_attr_fetch(), mid_frame_screen_bank_switch_splits_paper(), Default, Vec, TimexLoresMode, Ula48

### Community 67 - "camera.rs"
Cohesion: 0.13
Nodes (16): CameraPlugin, clamp01(), distance_for_crt_fill(), distance_matches_fill(), ease_in_out_cubic(), ease_out_cubic(), easing_endpoints(), lerp_eye_pullback_rise() (+8 more)

### Community 68 - "fuse.rs"
Cohesion: 0.19
Nodes (22): FuseEvent, Expected, fixtures_dir(), format_fuse_event(), fuse_all_vectors(), fuse_disasm_window(), fuse_mismatch_includes_disasm_at_start_pc(), fuse_smoke_nop() (+14 more)

### Community 69 - "PrefModel"
Cohesion: 0.25
Nodes (12): expand_main_rom_duplicates_boot_bank_without_stock(), expand_main_rom_image(), expected_rom_bytes(), MachineConfigError, resolve_main_rom(), PathBuf, Result, Vec (+4 more)

### Community 70 - "Bus48"
Cohesion: 0.10
Nodes (4): Bus128, Bus48, Option, Vec

### Community 71 - "dck.rs"
Cohesion: 0.19
Nodes (12): DckBank, DckBankId, DckChunkAccess, DckImage, parse_home_replace_and_empty_ram(), parse_spectrum_dock_header(), reject_truncated_pages(), reject_unknown_bank() (+4 more)

### Community 72 - ".new"
Cohesion: 0.19
Nodes (11): mid_line_border_128_uses_228_pitch(), mid_line_border_change_splits_scanline(), palette_rgb(), Self, stable_bank7_frame_uses_secondary_without_new_out(), timex_alt_file_uses_second_display(), timex_ext_colour_uses_8x1_attrs_from_alt(), timex_hires_attr_alt_reads_both_halves_from_alt() (+3 more)

### Community 73 - "FlatMem"
Cohesion: 0.13
Nodes (8): FlatMem, Io, Memory, NullIo, Box, Default, Self, FuseBus

### Community 74 - ".with_session_ref"
Cohesion: 0.09
Nodes (20): LastErrorResponse, Option, model_slug(), String, format_break_reason(), HardwareStatusResponse, HealthResponse, LastBreakResponse (+12 more)

### Community 75 - "Vec"
Cohesion: 0.24
Nodes (4): From, Vec, WatchesResponse, WatchSpec

### Community 76 - "joystick.rs"
Cohesion: 0.18
Nodes (13): apply_joystick(), clear_joystick_matrix(), cursor_uses_caps_and_5678(), JoystickMode, JoystickState, kempston_mask_roundtrip(), kempston_mode_sets_port_bits(), release_clears_previous_matrix() (+5 more)

### Community 77 - "auth_empty"
Cohesion: 0.17
Nodes (40): add_breakpoint(), add_port_watch(), add_watch(), BreakpointBody, disasm(), DisasmQuery, last_break(), list_breakpoints() (+32 more)

### Community 78 - "tick_emulator"
Cohesion: 0.27
Nodes (11): CameraLocked, host_hotkeys(), Assets, ButtonInput, Image, KeyCode, Option, Res (+3 more)

### Community 79 - "aperture_debug_enabled"
Cohesion: 0.83
Nodes (4): aperture_debug_enabled(), bright_debug_enabled(), env_flag(), hide_crt_enabled()

### Community 80 - "headless.rs"
Cohesion: 0.19
Nodes (17): bind_hybrid_headless_targets(), create_headless_render_image(), HeadlessRenderTargetHandle, HeadlessSize, rebuild_headless_render_target(), Assets, Commands, Entity (+9 more)

### Community 81 - "Self"
Cohesion: 0.31
Nodes (5): BeeperState, KeyScript, Default, Self, Vec

### Community 82 - "control_plane/src/present.rs"
Cohesion: 0.27
Nodes (13): compose_nearest_letterbox(), encode_rgba_png(), fit_letterboxes_wide(), fit_size(), host_display_rgba_len_checked(), nearest_scale2_doubles(), PresentMeta, PresentPanelSource (+5 more)

### Community 83 - "setup_room"
Cohesion: 0.24
Nodes (16): AssetServer, pbr_material(), Assets, Commands, Handle, Mesh, Option, Res (+8 more)

### Community 84 - "crt.rs"
Cohesion: 0.14
Nodes (16): ApertureDebugMarker, bulge_mesh_has_expected_vertex_count(), bulging_screen_mesh(), CrtPhosphorMaterial, CrtScreenTexture, CrtSpawnKit, overscan_sample_uv(), Handle (+8 more)

### Community 85 - "router"
Cohesion: 0.25
Nodes (18): router(), agent_api_dsk_rejects_non_plus3(), agent_api_hardware_attach_multiface_and_divmmc(), agent_api_host_display_and_window_unavailable(), agent_api_joystick_kempston_mask(), agent_api_keys_and_last_break(), agent_api_load_rom_by_path(), agent_api_media_insert_requires_machine() (+10 more)

### Community 86 - ".syncMatrix"
Cohesion: 0.33
Nodes (6): SpectrumKeymap, Bool, NSEvent, Set, UInt16, UInt32

### Community 87 - "upload_external_framebuffer"
Cohesion: 0.13
Nodes (13): ExternalFramebuffer, ExternalFramebufferPlugin, App, Assets, Default, Image, Option, Plugin (+5 more)

### Community 88 - "Keyboard"
Cohesion: 0.20
Nodes (4): Keyboard, keyboard_row(), Default, Self

### Community 89 - "TimexDock"
Cohesion: 0.20
Nodes (5): Default, Option, Self, TimexDock, TimexDockChunk

### Community 90 - "cpu.rs"
Cohesion: 0.25
Nodes (10): contend_read_timing_adds_wait_without_mr(), FuseEventKind, interrupt_clears_q_before_scf(), interrupt_im2_uncontended_is_19_t(), interrupt_while_halted_does_not_skip_redirected_pc(), interrupt_while_halted_resumes_after_halt(), nmi_vectors_to_0066_and_preserves_iff2(), B (+2 more)

### Community 91 - "flags.rs"
Cohesion: 0.30
Nodes (14): adc8(), add8(), and8(), cp8(), dec8_flags(), inc8_flags(), or8(), parity() (+6 more)

### Community 92 - "Agent Debug HTTP API"
Cohesion: 0.14
Nodes (15): SPEC_CHUM_AGENT_URL remote mode, spec-chum-debugging skill, agent_server crate, control_plane crate, spec-chum-agent, Agent Debug HTTP API, control_plane shared backend, HostSession (+7 more)

### Community 93 - "disasm.rs"
Cohesion: 0.06
Nodes (51): assert_z80test_passed(), code_block(), fixture_dir(), rom48_path(), Duration, Error, Path, PathBuf (+43 more)

### Community 94 - "KempstonMouse"
Cohesion: 0.22
Nodes (6): buttons_active_low(), delta_wraps_axes(), KempstonMouse, port_reads(), Option, Self

### Community 95 - "rzx.rs"
Cohesion: 0.25
Nodes (11): apply_input_byte(), apply_matrix_and_kempston(), minimal_rzx(), parse_input_frames(), FnMut, Path, Result, Self (+3 more)

### Community 96 - "mod.rs"
Cohesion: 0.21
Nodes (9): health_endpoint_ok(), model_post_parses_json(), ReadyError, Error, Option, serve(), test_app(), ReadySender (+1 more)

### Community 97 - "agent_embed.rs"
Cohesion: 0.17
Nodes (20): handle_mut(), Arc, c_void, Option, ParkingMutex, session_access(), SessionHandle, SessionInner (+12 more)

### Community 98 - "spawn"
Cohesion: 0.25
Nodes (13): EmbeddedServer, Arc, Option, Result, String, spawn(), spawn_fails_when_port_in_use(), spawn_from_env() (+5 more)

### Community 99 - "RoomPerf"
Cohesion: 0.19
Nodes (5): perf_log_enabled(), rolling_window_resets(), RoomPerf, Instant, Option

### Community 100 - "living_room/src/keymap.rs"
Cohesion: 0.29
Nodes (10): chord_for(), chord_suppresses_caps(), letter_digit(), matrix_from_bevy(), push_unique(), quote_shift_is_sym_p(), ButtonInput, KeyCode (+2 more)

### Community 101 - "quality.rs"
Cohesion: 0.22
Nodes (11): bloom_enabled(), env_truthy(), fxaa_enabled(), hybrid_enabled(), light_preset(), LightPreset, msaa_samples(), preset_label() (+3 more)

### Community 102 - "machine"
Cohesion: 0.50
Nodes (13): agent_server, app, bus, control_plane, debug_cli, formats, host_api, living_room (+5 more)

### Community 103 - "machine crate"
Cohesion: 0.20
Nodes (11): bus crate, formats crate, Hardware-faithful cycle-accurate accuracy, machine crate, tape crate, ula crate, z80 crate, synthetic_plus3_boot_marker (+3 more)

### Community 104 - "AppKit"
Cohesion: 0.27
Nodes (9): AppKit, CoreVideo, CSpecChumHost, Foundation, GameController, IOSurface, QuartzCore, SwiftUI (+1 more)

### Community 105 - "AppState"
Cohesion: 0.38
Nodes (22): attach_beta(), attach_divmmc(), attach_interface1(), attach_multiface(), eject_dck(), hardware_status(), insert_dck(), insert_mdr() (+14 more)

### Community 107 - "attach_crt_to_television"
Cohesion: 0.17
Nodes (15): animate_crt_params(), attach_crt_to_television(), CrtAttachedToTv, Assets, Commands, Entity, MeshMaterial3d, Option (+7 more)

### Community 108 - "media.rs"
Cohesion: 0.27
Nodes (18): load_dsk(), load_rom(), load_rzx(), load_snapshot(), load_trd(), HeaderMap, Json, Response (+10 more)

### Community 109 - "host_api crate"
Cohesion: 0.20
Nodes (10): ./scripts/check.sh, host_api crate, living_room crate, C ABI FFI-only policy, Guest framebuffer PNG export, Bevy 3D CRT living-room host, spec-chum-room, SCLD 512x192 hi-res modes (+2 more)

### Community 110 - "Full slow test suite"
Cohesion: 0.31
Nodes (11): ./scripts/run_slow_tests.sh, TDD expectations, Full slow test suite, CI z80doc job, ./scripts/run_system_tests.sh, Fuse Z80 test vectors, minfo.tap, Third-party system tests (+3 more)

### Community 111 - "living_room/src/lib.rs"
Cohesion: 0.27
Nodes (7): AssetPlugin, asset_plugin(), living_room_app(), resolve_asset_root(), App, PathBuf, main()

### Community 112 - "import_iosurface_texture"
Cohesion: 0.36
Nodes (6): import_iosurface_texture(), PresentIosurfaceError, c_void, RenderDevice, Result, Texture

### Community 113 - "routes/session.rs"
Cohesion: 0.22
Nodes (21): apply_config(), BorderBody, continue_execution(), ModelBody, reset(), HeaderMap, Json, Response (+13 more)

### Community 114 - "Kempston"
Cohesion: 0.28
Nodes (3): bits_active_high(), Kempston, Self

### Community 116 - "control_plane/src/window_capture.rs"
Cohesion: 0.11
Nodes (22): AtomicU32, CGImage, fit_size(), letterboxes_wide_window(), pillarboxes_tall_window(), Vec2, refresh_window_id_from_frame(), Frame (+14 more)

### Community 117 - "OpenMediaDialog"
Cohesion: 0.29
Nodes (6): FileDialogPlugin, OpenMediaDialog, App, Plugin, ResMut, run_open_dialog()

### Community 118 - "check_pr_reviews.sh"
Cohesion: 0.39
Nodes (7): append_bot_thread_from_comments(), apply_waiver_or_fail(), expect(), check_pr_reviews.sh script, classify_coderabbit_head_status(), parse_coderabbit_reset_minutes(), pr_review_cr_classify.sh script

### Community 119 - "Spec Chum"
Cohesion: 0.17
Nodes (15): ./scripts/fetch_roms.sh, Spec Chum, Dual-clock embed architecture, SpecChumMac SwiftUI shell, Multiface 1, Release process, Amstrad Lawson redistribution grant, ROM fetch policy (+7 more)

### Community 120 - "PrefsLock"
Cohesion: 0.29
Nodes (6): main(), Result, default_prefs_path(), PrefsLock, Drop, PathBuf

### Community 121 - "theme.rs"
Cohesion: 0.43
Nodes (5): apply(), apply_does_not_panic_on_default_context(), clear_color(), panel_fill_is_opaque(), Context

### Community 122 - "room_perf.rs"
Cohesion: 0.43
Nodes (6): main(), percentile_ms(), Duration, Vec, summarize(), varying_frame()

### Community 123 - "Snapshot128"
Cohesion: 0.25
Nodes (7): parse_z80_header(), Option, Snapshot128, Snapshot128Model, z80_machine_class(), Z80HeaderInfo, Z80MachineClass

### Community 124 - "custom_loader_tap"
Cohesion: 0.71
Nodes (6): checksum(), custom_loader_tap(), main(), make_code_tap(), CODE that `CALL 0556` (ROM LD-BYTES) for a following flag-0xC8 block. Models…, tap_block()

### Community 125 - "CodeRabbit merge gate"
Cohesion: 0.33
Nodes (6): ./scripts/check_pr_reviews.sh, auto_review.enabled false, coderabbit-review label, CodeRabbit on-demand reviews, CodeRabbit merge gate, Bot review threads gate

### Community 127 - "inspect.rs"
Cohesion: 0.12
Nodes (20): beta_inspect_from_128(), beta_inspect_from_48(), beta_json(), beta_json_includes_fdc_counters(), BetaInspect, Inspect, Machine, opt_u8() (+12 more)

### Community 128 - "fb_scale.rs"
Cohesion: 0.40
Nodes (4): blit_to_crt(), dims_from_rgba_len(), Option, scale_hires_paper_to_crt()

### Community 130 - "fetch_roms.sh"
Cohesion: 0.67
Nodes (5): checkout_sparse_repo(), copy_dir_roms(), copy_rom(), count_managed_roms(), fetch_roms.sh script

### Community 131 - "app crate"
Cohesion: 0.33
Nodes (6): app crate, debug_cli crate, spec-chum-debug, egui/eframe primary host, Spec Chum.app release bundle, Release workflow

### Community 132 - "Flash-load at LD-BYTES 0x056C"
Cohesion: 0.40
Nodes (5): Flash-load convenience exception, Flash-load at LD-BYTES 0x056C, type-load subcommand, attr_mark.tap, custom_loader.tap

### Community 133 - "PrefAyStereo"
Cohesion: 0.27
Nodes (3): PrefAyStereo, AyStereoMode, JoystickMode

### Community 134 - "fetch_system_tests.sh"
Cohesion: 0.80
Nodes (4): fetch(), fetch_system_tests.sh script, sha256_of(), verify_sha()

### Community 135 - "room_perf_matrix.sh"
Cohesion: 0.50
Nodes (4): run_one(), RUST_LOG, room_perf_matrix.sh script, SPEC_CHUM_ROOM_PERF_SOFT

### Community 136 - "trace_dump"
Cohesion: 0.27
Nodes (13): HeaderMap, Json, Option, Query, Response, State, String, set_trace_categories() (+5 more)

### Community 137 - "UiPreferences"
Cohesion: 0.18
Nodes (10): model_rom_path_key(), pref_model_slug_matches_json_snake_case(), BTreeMap, Default, Option, String, UserMachineConfig, Vec (+2 more)

### Community 138 - "CrtPlugin"
Cohesion: 0.50
Nodes (3): CrtPlugin, App, Plugin

### Community 139 - "HybridPlugin"
Cohesion: 0.50
Nodes (3): HybridPlugin, App, Plugin

### Community 140 - "Test tier matrix"
Cohesion: 0.22
Nodes (9): Hardware-faithful vs convenience, Known non-blocking noise, Lint and check inventory, Related docs, ROM and fixture skip policy, Test tier matrix, Testing and quality gates, What “provable” means (+1 more)

### Community 141 - ".inspect"
Cohesion: 0.60
Nodes (5): floating_bus_byte(), floating_bus_byte_128(), floating_bus_byte_48(), floating_bus_params(), Option

### Community 142 - "UiOverlayPlugin"
Cohesion: 0.50
Nodes (3): App, Plugin, UiOverlayPlugin

### Community 143 - "check_crates.sh"
Cohesion: 0.67
Nodes (3): infer_crates(), RUSTFLAGS, check_crates.sh script

### Community 146 - "build_macos_app.sh"
Cohesion: 0.33
Nodes (5): CFLAGS, CXXFLAGS, MACOSX_DEPLOYMENT_TARGET, build_macos_app.sh script, SPEC_CHUM_ROOT

### Community 171 - "HostSession"
Cohesion: 0.07
Nodes (10): HostAccess, Deref, DerefMut, MutexGuard, HostSession, open_fixture_tap_progress_and_audio_pcm(), Into, Machine (+2 more)

### Community 172 - ".setFlashLoad"
Cohesion: 0.18
Nodes (5): LoadKeyScript, Step, Bool, Int, UInt32

### Community 173 - "RomLoadError"
Cohesion: 0.25
Nodes (6): multiface_kempston_on_in_1f_while_attached(), rom_load_wrong_size_is_typed(), RomLoadError, Error, Result, Result

### Community 174 - "health"
Cohesion: 0.57
Nodes (6): health(), last_error(), HeaderMap, Response, State, status()

### Community 175 - "audio.rs"
Cohesion: 0.11
Nodes (26): AtomicUsize, audio_capture_enabled(), AudioMuted, AudioOut, AudioPlugin, AudioStream, DcBlock, fill_output() (+18 more)

### Community 176 - ".load_rom_bytes_with_overrides"
Cohesion: 0.15
Nodes (7): load_128_or_plus3_rom(), render_frame_pcm(), rom_search_roots(), BTreeMap, PathBuf, Vec, workspace_root()

### Community 177 - ".body"
Cohesion: 0.10
Nodes (7): App, URL, URL, URL, SpecChumMacApp, .body, Scene

### Community 178 - "is_screen_tri"
Cohesion: 0.67
Nodes (3): is_screen_tri(), main(), True only for the painted glass face (inner aperture), not either bezel.

### Community 182 - "MachineConfigEditorView"
Cohesion: 0.07
Nodes (26): DebugInspectorView, .body, GlassBarBackground, View, Bool, Int32, UInt16, UInt32 (+18 more)

### Community 183 - "NSEvent"
Cohesion: 0.14
Nodes (5): KempstonMouseTracking, Bool, Int, NSEvent, NSEvent

### Community 184 - "crt_phosphor_local"
Cohesion: 0.67
Nodes (4): bottom_adjust_keeps_top_edge(), crt_phosphor_local(), crt_screen_world_center(), Vec3

### Community 185 - "input.rs"
Cohesion: 0.27
Nodes (15): get_prefs(), JoystickBody, KeyAction, KeysBody, MouseBody, patch_prefs(), HeaderMap, Json (+7 more)

### Community 186 - ".share_host"
Cohesion: 0.50
Nodes (4): HostSlot, Arc, Box, ParkingMutex

### Community 190 - "RoomPlugin"
Cohesion: 0.50
Nodes (3): RoomPlugin, App, Plugin

## Knowledge Gaps
- **156 isolated node(s):** `PackageDescription`, `Notification.Name`, `.isMenuTracking`, `.body`, `.hasBeta` (+151 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 767 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **25 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `HostSession` connect `HostSession` to `FramebufferMeta`, `host_api/src/ffi.rs`, `agent_embed.rs`, `.recompose_input`, `AgentClient`, `HostError`, `HostViewState`, `.with_host_mut`, `.with_session_ref`, `joystick.rs`, `ControlPlane`, `service.rs`, `.load_rom_bytes_with_overrides`, `.open_tape`, `src/session.rs`, `ModelId`, `.share_host`, `EmulatorHost`?**
  _High betweenness centrality (0.251) - this node is a cross-community bridge._
- **Why does `Machine` connect `Machine` to `Cpu`, `.step_once`, `Ula48`, `Bus48`, `machine/src/lib.rs`, `.new_48k`, `joystick.rs`, `Kempston`, `custom_loader_matrix_models_instant_and_ear`, `Debugger`, `.cpu_mut`, `BusPlus3`, `Ay8912`, `KempstonMouse`?**
  _High betweenness centrality (0.217) - this node is a cross-community bridge._
- **Why does `EmulatorHost` connect `EmulatorHost` to `HostSession`, `ui_overlay.rs`, `OpenMediaDialog`, `tick_emulator`?**
  _High betweenness centrality (0.195) - this node is a cross-community bridge._
- **Are the 4 inferred relationships involving `HostBridge` (e.g. with `.livingRoomToolbar` and `TapeAudioPlayer`) actually correct?**
  _`HostBridge` has 4 INFERRED edges - model-reasoned connections that need verification._
- **What connects `PackageDescription`, `Notification.Name`, `.isMenuTracking` to the rest of the system?**
  _156 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `host_api/src/ffi.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.07743362831858407 - nodes in this community are weakly interconnected._
- **Should `Plus3Fdc` be split into smaller, more focused modules?**
  _Cohesion score 0.0514018691588785 - nodes in this community are weakly interconnected._