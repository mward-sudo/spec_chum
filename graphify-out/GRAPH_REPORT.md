# Graph Report - e5j4  (2026-09-04)

## Corpus Check
- 167 files · ~217,384 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 4126 nodes · 11442 edges · 175 communities (141 shown, 28 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 282 edges (avg confidence: 0.83)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `f5f2129e`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- routes.rs
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
- session.rs
- living_room/src/ffi.rs
- hybrid_state_machine
- .cpu
- .body
- image_copy.rs
- BusPlus3
- .takeLastError
- TapeAudioPlayer
- machine_config.rs
- UiPreferences
- Model
- DskImage
- .step_once
- LivingRoomNSView
- audio.rs
- formats/src/lib.rs
- prefs.rs
- PrefModelSlug
- SpecChumApp
- .new
- Cpu
- RomSetupSlot
- machine/src/lib.rs
- Multiface1
- service.rs
- .load_rom_bytes_with_overrides
- Ay8912
- snow.rs
- bus/src/lib.rs
- custom_loader_matrix_models_instant_and_ear
- ui_overlay.rs
- interface1_opcode_fetch_pages_shadow_rom
- HeadlessRoom
- Debugger
- ModelId
- .host_mut
- TimexScld
- EmulatorHost
- inspect.rs
- FormatError
- glow.rs
- PresentTarget
- .add_t
- .new
- Ula48
- camera.rs
- fuse.rs
- .recompose_input
- Bus48
- dck.rs
- .new
- FlatMem
- Self
- Keyboard
- joystick.rs
- .with_host_mut
- Registers
- crt.rs
- headless.rs
- TrdImage
- Option
- setup_room
- setup_crt_resources
- router
- .syncMatrix
- upload_external_framebuffer
- Bool
- TimexDock
- cpu.rs
- flags.rs
- Agent Debug HTTP API
- room_probe.rs
- KempstonMouse
- rzx.rs
- .onLivingRoomDisplayTick
- agent_embed.rs
- Bus128
- RoomPerf
- living_room/src/keymap.rs
- quality.rs
- machine
- machine crate
- MachineConfigEditorView
- z80test.rs
- HostSession
- attach_crt_to_television
- NSEvent
- host_api crate
- Full slow test suite
- living_room/src/lib.rs
- control_plane shared backend
- .body
- Kempston
- CodingKeys
- error.rs
- file_dialog.rs
- check_pr_reviews.sh
- Spec Chum
- Result
- theme.rs
- room_perf.rs
- import_iosurface_texture
- custom_loader_tap
- CodeRabbit merge gate
- tick_emulator
- disasm.rs
- fb_scale.rs
- PrefModel
- fetch_roms.sh
- app crate
- Flash-load at LD-BYTES 0x056C
- Self
- fetch_system_tests.sh
- room_perf_matrix.sh
- .write
- FramebufferMeta
- CrtPlugin
- HybridPlugin
- PresentBlitPlugin
- parse_model
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
- SpectrumDisplayView
- ./scripts/check_crates.sh
- gh stack workflow
- .open_tape
- mid_line_border_128_uses_228_pitch
- Vec
- .fmt

## God Nodes (most connected - your core abstractions)
1. `HostBridge` - 208 edges
2. `HostSession` - 134 edges
3. `Machine` - 125 edges
4. `ControlPlane` - 105 edges
5. `AppState` - 79 edges
6. `session_mut()` - 66 edges
7. `HostError` - 65 edges
8. `Cpu` - 63 edges
9. `LivingRoomNSView` - 59 edges
10. `BetaDisk` - 55 edges

## Surprising Connections (you probably didn't know these)
- `control_plane crate` --references--> `Agent Debug HTTP API`  [EXTRACTED]
  AGENTS.md → docs/AGENT_DEBUG_API.md
- `formats crate` --references--> `synthetic_plus3_boot_marker`  [INFERRED]
  AGENTS.md → tests/fixtures/plus3/README.md
- `ula crate` --references--> `ulatest3.tap`  [INFERRED]
  AGENTS.md → tests/fixtures/system/README.md
- `z80 crate` --references--> `Fuse Z80 test vectors`  [EXTRACTED]
  AGENTS.md → tests/fixtures/fuse/README.md
- `Patrik Rak z80test` --references--> `Spec Chum`  [EXTRACTED]
  tests/fixtures/z80test/README.md → README.md

## Import Cycles
- 2-file cycle: `crates/z80/src/cpu.rs -> crates/z80/src/opcodes.rs -> crates/z80/src/cpu.rs`
- 2-file cycle: `crates/formats/src/dck.rs -> crates/formats/src/lib.rs -> crates/formats/src/dck.rs`
- 2-file cycle: `crates/formats/src/dsk.rs -> crates/formats/src/lib.rs -> crates/formats/src/dsk.rs`
- 2-file cycle: `crates/formats/src/lib.rs -> crates/formats/src/mdr.rs -> crates/formats/src/lib.rs`
- 2-file cycle: `crates/formats/src/lib.rs -> crates/formats/src/rzx.rs -> crates/formats/src/lib.rs`
- 2-file cycle: `crates/formats/src/lib.rs -> crates/formats/src/trd.rs -> crates/formats/src/lib.rs`
- 3-file cycle: `crates/formats/src/dsk.rs -> crates/formats/src/lib.rs -> crates/formats/src/fdc.rs -> crates/formats/src/dsk.rs`

## Communities (175 total, 28 thin omitted)

### Community 0 - "routes.rs"
Cohesion: 0.06
Nodes (132): add_breakpoint(), add_port_watch(), add_watch(), api_error(), apply_config(), AppState, attach_beta(), attach_divmmc() (+124 more)

### Community 1 - "host_api/src/ffi.rs"
Cohesion: 0.08
Nodes (107): break_reason_code(), clear_last_error(), ffi_bad_model_returns_null(), ffi_create_destroy_and_run(), ffi_debug_dump_json_and_peek_null(), ffi_joystick_mode_rejects_truncated_overflow(), ffi_mouse_delta_and_buttons_smoke(), heap_cstring() (+99 more)

### Community 2 - "Plus3Fdc"
Cohesion: 0.09
Nodes (33): command_len(), drain_result(), feed_format_track(), feed_read_data(), format_track_no_image_returns_abnormal_nd(), format_track_out_of_range_returns_abnormal_nd(), format_track_replaces_sectors_on_disk(), format_track_write_protect_returns_nw() (+25 more)

### Community 3 - "trace/src/lib.rs"
Cohesion: 0.06
Nodes (59): AsRef, BitOr, BitOrAssign, BufWriter, append_flushes_events_to_trace_file(), AppendSink, categories(), Category (+51 more)

### Community 4 - "rom.rs"
Cohesion: 0.09
Nodes (77): expected_main_rom_bytes(), exrom_available(), exrom_available_in(), exrom_candidates(), install_rom_slot(), install_rom_slot_copies_to_expected_path(), install_rom_slot_validates_size(), main_rom_available() (+69 more)

### Community 5 - "HostBridge"
Cohesion: 0.04
Nodes (30): HostBridge, .experienceLoad, .hasBeta, .hasTimexDock, .instantLoad, .isCustomConfigActive, .kempstonMouse, .livingRoomMode (+22 more)

### Community 6 - "AgentClient"
Cohesion: 0.08
Nodes (32): Agent, AgentClient, AuthRequest, Option, Result, Self, String, Value (+24 more)

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
Nodes (15): contention_delay(), contention_delay_128(), contention_delay_48(), contention_delay_params(), floating_bus_byte(), floating_bus_byte_128(), floating_bus_byte_48(), floating_bus_params() (+7 more)

### Community 11 - "system_tests.rs"
Cohesion: 0.09
Nodes (70): amstrad_plus2a_no_snow_with_i_40(), amstrad_plus3_no_snow_with_i_40(), assert_contended_nop_pattern(), assert_no_snow_with_i_40(), assert_pattern(), assert_screen_has(), assert_snow_disrupts_testcard(), azesmbog_loads_and_paints() (+62 more)

### Community 12 - ".new_48k"
Cohesion: 0.18
Nodes (40): apply_sna48_sets_pc_ram_and_border(), apply_z80_snapshot48_sets_pc_ram_and_border(), attr_mark_code_ok(), attr_mark_ear_load_quotes_code_succeeds_at_speed_10(), attr_mark_experience_load_succeeds(), attr_mark_fixture_flash_loads_code_bytes(), attr_mark_load_path_dumps_trace_on_failure(), attr_mark_load_path_must_succeed() (+32 more)

### Community 13 - "ControlPlane"
Cohesion: 0.09
Nodes (7): apply_prefs_to_session(), ControlPlane, ApiResult, Mutex, Path, SharedHostView, UserMachineConfig

### Community 14 - "BetaDisk"
Cohesion: 0.08
Nodes (47): beta_fdc_emits_disk_trace(), BetaDisk, disk_not_ready_without_image(), drive_b_is_not_ready(), feed_write_track_sector(), force_interrupt_clears_drq(), m1_48k_pages_at_3c00_128k_does_not(), m1_does_not_unpage_while_executing_trdos_rom() (+39 more)

### Community 15 - "tzx.rs"
Cohesion: 0.10
Nodes (30): active_pulse_counters_are_block_relative(), advance_clears_high_ear_on_exact_final_pulse_end(), advance_clears_playing_on_exact_final_pulse_boundary(), append_pure_data(), append_standard_block(), append_turbo_block(), empty_tzx_reports_zero_blocks(), minimal_tzx_standard() (+22 more)

### Community 16 - "DivMmc"
Cohesion: 0.11
Nodes (23): automap_entry_and_exit_with_eeprom(), automap_ignored_without_eeprom_or_mapram(), control_port_conmem_shows_ram_page(), DivMmc, eeprom_accepts_larger_image_prefix(), mapram_is_sticky_across_control_writes(), mapram_uses_page_3_in_lower_8k(), Default (+15 more)

### Community 17 - "SpectrumNSView"
Cohesion: 0.12
Nodes (14): SpectrumNSView, .acceptsFirstResponder, .canBecomeKeyView, .focusRingMaskBounds, .isFlipped, Any, Bool, Int (+6 more)

### Community 18 - "host_api/src/keymap.rs"
Cohesion: 0.09
Nodes (36): chord_for(), letter_digit(), modifier_keys(), mods_shift(), punct_chord(), quote_alone_is_symbol_7(), quote_shift_is_symbol_p(), Key (+28 more)

### Community 19 - "Machine"
Cohesion: 0.06
Nodes (13): TimexDockError, Machine, Model, plus2a_model_has_no_disk_and_rejects_dsk(), plus3_loader_dos_boot_runs_titled_marker(), plus3_loader_load_disk_runs_basic_marker(), plus3_loader_talks_to_fdc_on_data_disk(), plus3_model_keeps_disk_interface() (+5 more)

### Community 20 - "apply_zoom_camera"
Cohesion: 0.09
Nodes (37): Bloom, anim_eases_toward_target(), apply_zoom_camera(), CameraZoom, CrtLookBlend, IntroSkipRequest, nudge_clamps(), pose_at_zoom() (+29 more)

### Community 21 - "session.rs"
Cohesion: 0.16
Nodes (33): attach_beta_on_48k_and_reject_plus3(), border_toggle_resizes_framebuffer(), cursor_left_via_joystick_applies_caps_five(), dims(), joystick_kempston_mask_reaches_port(), kempston_arrow_left_does_not_pollute_matrix(), kempston_mouse_ports_after_synthetic_deltas(), load_dsk_rejects_non_plus3() (+25 more)

### Community 22 - "living_room/src/ffi.rs"
Cohesion: 0.18
Nodes (36): catch_const_u8(), catch_int(), catch_ptr(), catch_uint(), clear_last_error(), room_mut(), RoomHandle, c_char (+28 more)

### Community 23 - "hybrid_state_machine"
Cohesion: 0.16
Nodes (41): Camera, Children, LivingRoomCamera, apply_hybrid_display(), bind_present_target(), ensure_plate_image(), hybrid_state_machine(), HybridBakeCamera (+33 more)

### Community 24 - ".cpu"
Cohesion: 0.17
Nodes (33): apply_trdos_find_boot_native_abi(), apply_trdos_run_native_abi(), custom_loader_ok(), debug_trdos_line_new_handoff(), debug_trdos_run_pc_trace(), ensure_trdos_beta128_prog(), enter_128k_basic_from_menu(), enter_trdos_command_mode() (+25 more)

### Community 25 - ".body"
Cohesion: 0.09
Nodes (23): activateSpecChum(), AppDelegate, Notification.Name, .body, .flatSpectrumChrome, .livingRoomChrome, DebugInspectorView, FocusSpectrumView (+15 more)

### Community 26 - "image_copy.rs"
Cohesion: 0.09
Nodes (36): Buffer, despawn_image_copiers(), drain_copied_frames(), image_copy_driver(), image_copy_extract(), ImageCopier, ImageCopiers, ImageCopyPlugin (+28 more)

### Community 27 - "BusPlus3"
Cohesion: 0.10
Nodes (19): BusPlus3, contended_banks_are_4_through_7(), fdc_motor_bit_on_1ffd_affects_st3(), fdc_read_data_protocol_via_ports(), is_contended_bank_plus3(), lock_blocks_both_ports(), no_floating_bus(), out_7ffd_address_does_not_hit_1ffd() (+11 more)

### Community 29 - "TapeAudioPlayer"
Cohesion: 0.12
Nodes (21): AudioCaptureFile, AudioLog, Stats, Bool, Double, Float, Int, String (+13 more)

### Community 30 - "machine_config.rs"
Cohesion: 0.11
Nodes (37): AppliedConfig, apply_builtin_rom_when_no_override(), apply_diagrom_succeeds_on_128k_class_models(), apply_diagrom_succeeds_on_16k_class_models(), apply_diagrom_succeeds_on_plus3_with_16k_rom(), apply_rejects_bad_custom_rom_size(), apply_user_config(), build_machine() (+29 more)

### Community 31 - "UiPreferences"
Cohesion: 0.17
Nodes (10): model_rom_path_key(), pref_model_slug_matches_json_snake_case(), BTreeMap, Default, Option, String, UserMachineConfig, Vec (+2 more)

### Community 32 - "Model"
Cohesion: 0.08
Nodes (20): Model, .id, pentagon128, .prefSlug, .requiresUserProvidedRoms, .romAvailable, .shortTitle, spectrum128 (+12 more)

### Community 33 - "DskImage"
Cohesion: 0.14
Nodes (20): cpm_dir_entry(), DskImage, find_id_matches_r_without_chrn_c(), multi_sector_dsk_lookup(), parse_and_read_sector(), parse_track(), plus3_basic_poke_marker(), plus3_cpm_chs() (+12 more)

### Community 34 - ".step_once"
Cohesion: 0.07
Nodes (32): emit_contend_sampled(), FrameAudio, manual_read_track1_sector1(), mem_port_watch(), MemIo128, MemIo48, MemIoPlus3, model_plus2_tags_grey_plus2() (+24 more)

### Community 35 - "LivingRoomNSView"
Cohesion: 0.09
Nodes (18): LivingRoomNSView, .acceptsFirstResponder, .canBecomeKeyView, .focusRingMaskBounds, .host, .isFlipped, Any, Bool (+10 more)

### Community 36 - "audio.rs"
Cohesion: 0.11
Nodes (26): AtomicUsize, audio_capture_enabled(), AudioMuted, AudioOut, AudioPlugin, AudioStream, DcBlock, fill_output() (+18 more)

### Community 37 - "formats/src/lib.rs"
Cohesion: 0.14
Nodes (26): parse_sna128_regs_banks_pc(), parse_sna128_when_paged_is_bank5(), parse_z80_rejects_undersized_extended_header(), parse_z80_v1_compressed_regs_and_ram(), parse_z80_v1_uncompressed_regs_and_ram(), parse_z80_v2_128_banks_and_7ffd(), parse_z80_v2_pages_land_at_48k_addresses(), parse_z80_v3_hw8_is_plus2a() (+18 more)

### Community 38 - "prefs.rs"
Cohesion: 0.11
Nodes (24): main(), Result, corrupt_file_falls_back_to_defaults(), custom_configs_round_trip(), default_prefs_path(), load_prefs(), load_prefs_unlocked(), missing_file_falls_back_to_defaults() (+16 more)

### Community 39 - "PrefModelSlug"
Cohesion: 0.07
Nodes (33): JoystickMode, cursor, .id, kempston, sinclairLeft, sinclairRight, .title, PrefAyStereoSlug (+25 more)

### Community 40 - "SpecChumApp"
Cohesion: 0.08
Nodes (19): App, BTreeMap, Context, Debug, Formatter, Frame, Instant, Machine (+11 more)

### Community 41 - ".new"
Cohesion: 0.14
Nodes (16): continue_and_eject_require_machine(), health_and_inspect_after_rom_load(), mouse_requires_kempston_pref_then_accepts_input(), prefs_apply_after_rom_load(), prefs_patch_round_trip(), rom48(), Arc, Default (+8 more)

### Community 42 - "Cpu"
Cohesion: 0.18
Nodes (31): add16(), adc16(), block_cp(), block_in(), block_ld(), block_out(), condition(), daa() (+23 more)

### Community 43 - "RomSetupSlot"
Cohesion: 0.11
Nodes (23): AppKit, JsonRoot, JsonSlot, RomSetupCodec, RomSetupPayload, RomSetupSlot, .sizeHint, .statusColor (+15 more)

### Community 44 - "machine/src/lib.rs"
Cohesion: 0.09
Nodes (29): advance_frame_t(), apply_snapshot128_plus3_applies_1ffd(), apply_snapshot128_z80_pages_and_7ffd(), beta_reads_synthetic_boot_basic_into_ram(), beta_trdos_rom_loop_reads_trd_sector_into_ram(), beta_write_track_via_synthetic_rom(), custom_loader_tap(), m1_refresh_pentagon128_skips_snow() (+21 more)

### Community 45 - "Multiface1"
Cohesion: 0.12
Nodes (15): button_pages_on_nmi_vector(), in_9f_pages_in_in_1f_pages_out(), load_rom_size_check(), Multiface1, multiface1_port_match(), out_1f_clears_nmi_pending_without_unpaging(), out_3f_is_not_mf1_decode(), reset_clears_paging_keeps_ram() (+7 more)

### Community 46 - "service.rs"
Cohesion: 0.09
Nodes (18): format_break_reason(), HardwareStatusResponse, HealthResponse, last_error_records_failures(), LastBreakResponse, LastErrorRecord, MemoryMapResponse, MemoryRegion (+10 more)

### Community 47 - ".load_rom_bytes_with_overrides"
Cohesion: 0.15
Nodes (7): load_128_or_plus3_rom(), render_frame_pcm(), rom_search_roots(), BTreeMap, PathBuf, Vec, workspace_root()

### Community 48 - "Ay8912"
Cohesion: 0.14
Nodes (15): acb_vs_abc_pan_differs(), acb_vs_abc_swap_b_and_c_pans(), Ay8912, ay_channel_b_only(), envelope_level(), envelope_write_restarts(), mixer_mute_silence(), mono_stereo_matches_sample_mono() (+7 more)

### Community 49 - "snow.rs"
Cohesion: 0.14
Nodes (14): corrupt_128_uses_alternate_bank_source(), corrupt_row32_col0_r_zero_not_skipped(), corrupt_skipped_when_r_matches_addr_lo(), corrupt_uses_refresh_low_byte_not_display(), double_duplicates_previous_column(), i_pointed_bank_128(), pattern_at_phase(), Option (+6 more)

### Community 50 - "bus/src/lib.rs"
Cohesion: 0.15
Nodes (22): beta_ports_when_trdos_paged_via_bus48(), beta_trdos_rom_overlays_when_paged(), bus128_m1_pages_trdos_at_3d00_not_3c00(), contend_128_differs_from_48_at_paper_start(), divmmc_automap_via_notify_m1(), divmmc_conmem_overlays_via_bus48(), divmmc_control_beats_interface1_on_shared_e3(), divmmc_eeprom_fixture_automaps_when_present() (+14 more)

### Community 51 - "custom_loader_matrix_models_instant_and_ear"
Cohesion: 0.19
Nodes (16): attr_mark_load_matrix_models_and_speeds(), attr_mark_type_load_128k_flash(), attr_mark_type_load_plus3_flash(), ay_frame_audio_nonzero_when_tone_programmed(), boggit_side1_matrix_when_present(), custom_loader_matrix_models_instant_and_ear(), inspect_128k_paging(), MachineBuildError (+8 more)

### Community 52 - "ui_overlay.rs"
Cohesion: 0.13
Nodes (27): BackgroundColor, Changed, ChildSpawnerCommands, CameraIntro, chrome_button(), chrome_buttons(), ChromeAction, host_cmd_shortcuts() (+19 more)

### Community 53 - "interface1_opcode_fetch_pages_shadow_rom"
Cohesion: 0.40
Nodes (3): interface1_opcode_fetch_pages_shadow_rom(), interface1_rom_load_skips_cleanly_when_missing(), Interface1Error

### Community 54 - "HeadlessRoom"
Cohesion: 0.13
Nodes (7): HeadlessRoom, rebuild_headless_render_target(), Debug, Formatter, Result, Self, String

### Community 55 - "Debugger"
Cohesion: 0.12
Nodes (8): Debugger, Cell, Default, Option, Self, Vec, Watch, WatchHook

### Community 56 - "ModelId"
Cohesion: 0.18
Nodes (25): canonical_persist_path(), install_model_rom(), model_requires_user_rom(), model_rom_available(), model_rom_paths_snapshot(), pentagon_rom_setup_has_user_slots(), persisted_path_wins_over_missing_workspace(), rom_setup_json() (+17 more)

### Community 57 - ".host_mut"
Cohesion: 0.19
Nodes (3): EmulatorSession, Path, tzx_standard_inserts_as_paused_tap()

### Community 58 - "TimexScld"
Cohesion: 0.12
Nodes (8): altmembank_and_chunk_bits(), port_f4_latches(), port_ff_read_returns_last_write(), Option, Self, screen_mode_and_int_disable_from_port_ff(), TimexScld, TimexScreenMode

### Community 59 - "EmulatorHost"
Cohesion: 0.12
Nodes (14): EmulatorHost, HostPlugin, model_label(), App, Debug, Duration, Formatter, PathBuf (+6 more)

### Community 60 - "inspect.rs"
Cohesion: 0.14
Nodes (18): beta_inspect_from_128(), beta_inspect_from_48(), beta_json(), beta_json_includes_fdc_counters(), BetaInspect, Inspect, Machine, opt_u8() (+10 more)

### Community 61 - "FormatError"
Cohesion: 0.19
Nodes (17): decode_z80_page(), decode_z80_v1(), FormatError, load_z80_v2_pages_128(), load_z80_v2_pages_48(), parse_z80_header(), Display, Error (+9 more)

### Community 62 - "glow.rs"
Cohesion: 0.11
Nodes (21): CrtPhosphor, CrtFillLight, FrameGlow, GlowDriven, GlowPlugin, IncandescentLamp, red_border_dominates_glow(), App (+13 more)

### Community 63 - "PresentTarget"
Cohesion: 0.11
Nodes (20): blit_to_present(), extract_present_target(), ExtractedPresent, PresentTarget, Arc, Commands, Debug, Extract (+12 more)

### Community 64 - ".add_t"
Cohesion: 0.23
Nodes (5): Cpu, Option, Vec, I, M

### Community 65 - ".new"
Cohesion: 0.13
Nodes (26): arrow_left_maps_joystick_kempston_and_cursor_mode(), debug_window_smoke_headless(), egui_menu_smoke_without_window(), emulator_session_uses_host_session(), gui_and_control_plane_share_live_session(), HostSlot, load_snapshot48_switches_from_128k(), load_snapshot48_switches_from_plus3() (+18 more)

### Community 66 - "Ula48"
Cohesion: 0.16
Nodes (9): bank_switch_between_bitmap_and_attr_fetch(), mid_frame_screen_bank_switch_splits_paper(), Default, Vec, stable_bank7_frame_uses_secondary_without_new_out(), TimexLoresMode, Ula48, SnowCellKind (+1 more)

### Community 67 - "camera.rs"
Cohesion: 0.13
Nodes (16): CameraPlugin, clamp01(), distance_for_crt_fill(), distance_matches_fill(), ease_in_out_cubic(), ease_out_cubic(), easing_endpoints(), lerp_eye_pullback_rise() (+8 more)

### Community 68 - "fuse.rs"
Cohesion: 0.21
Nodes (21): FuseEvent, Expected, fixtures_dir(), format_fuse_event(), fuse_all_vectors(), fuse_disasm_window(), fuse_mismatch_includes_disasm_at_start_pc(), fuse_smoke_nop() (+13 more)

### Community 69 - ".recompose_input"
Cohesion: 0.14
Nodes (4): JoystickMode, Model, Self, UserMachineConfig

### Community 71 - "dck.rs"
Cohesion: 0.19
Nodes (12): DckBank, DckBankId, DckChunkAccess, DckImage, parse_home_replace_and_empty_ram(), parse_spectrum_dock_header(), reject_truncated_pages(), reject_unknown_bank() (+4 more)

### Community 72 - ".new"
Cohesion: 0.20
Nodes (10): framebuffer_dims(), palette_rgb(), Self, timex_alt_file_uses_second_display(), timex_ext_colour_uses_8x1_attrs_from_alt(), timex_hires_attr_alt_reads_both_halves_from_alt(), timex_hires_double_col_uses_alt_only(), timex_hires_ink_paper() (+2 more)

### Community 73 - "FlatMem"
Cohesion: 0.13
Nodes (8): FlatMem, Io, Memory, NullIo, Box, Default, Self, FuseBus

### Community 74 - "Self"
Cohesion: 0.35
Nodes (5): BeeperState, KeyScript, Default, Self, Vec

### Community 75 - "Keyboard"
Cohesion: 0.20
Nodes (4): Keyboard, keyboard_row(), Default, Self

### Community 76 - "joystick.rs"
Cohesion: 0.20
Nodes (13): apply_joystick(), clear_joystick_matrix(), cursor_uses_caps_and_5678(), JoystickMode, JoystickState, kempston_mask_roundtrip(), kempston_mode_sets_port_bits(), release_clears_previous_matrix() (+5 more)

### Community 78 - "Registers"
Cohesion: 0.11
Nodes (7): pairs_round_trip(), r_preserves_bit7(), Registers, Display, Formatter, Result, Self

### Community 79 - "crt.rs"
Cohesion: 0.15
Nodes (12): aperture_debug_enabled(), ApertureDebugMarker, bottom_adjust_keeps_top_edge(), bright_debug_enabled(), crt_phosphor_local(), crt_screen_world_center(), CrtAttachedToTv, env_flag() (+4 more)

### Community 80 - "headless.rs"
Cohesion: 0.18
Nodes (17): bind_hybrid_headless_targets(), create_headless_render_image(), HeadlessRenderTargetHandle, HeadlessSize, Assets, c_void, Commands, Entity (+9 more)

### Community 81 - "TrdImage"
Cohesion: 0.20
Nodes (12): format_track_clears_and_sets_sectors(), parse_and_read_sector(), Option, Path, Result, Self, Vec, synthetic_trd() (+4 more)

### Community 82 - "Option"
Cohesion: 0.26
Nodes (14): compose_nearest_letterbox(), encode_rgba_png(), fit_letterboxes_wide(), fit_size(), host_display_rgba_len_checked(), nearest_scale2_doubles(), PresentMeta, PresentPanelSource (+6 more)

### Community 83 - "setup_room"
Cohesion: 0.17
Nodes (19): AssetServer, pbr_material(), RoomPlugin, App, Assets, Commands, Handle, Mesh (+11 more)

### Community 84 - "setup_crt_resources"
Cohesion: 0.17
Nodes (16): bulge_mesh_has_expected_vertex_count(), bulging_screen_mesh(), CrtPhosphorMaterial, CrtScreenTexture, CrtSpawnKit, Assets, Commands, Handle (+8 more)

### Community 85 - "router"
Cohesion: 0.25
Nodes (18): router(), agent_api_dsk_rejects_non_plus3(), agent_api_hardware_attach_multiface_and_divmmc(), agent_api_host_display_and_window_unavailable(), agent_api_joystick_kempston_mask(), agent_api_keys_and_last_break(), agent_api_load_rom_by_path(), agent_api_media_insert_requires_machine() (+10 more)

### Community 86 - ".syncMatrix"
Cohesion: 0.37
Nodes (6): SpectrumKeymap, Bool, NSEvent, Set, UInt16, UInt32

### Community 87 - "upload_external_framebuffer"
Cohesion: 0.13
Nodes (13): ExternalFramebuffer, ExternalFramebufferPlugin, App, Assets, Default, Image, Option, Plugin (+5 more)

### Community 88 - "Bool"
Cohesion: 0.12
Nodes (8): .joystickMode, LoadKeyScript, Step, Bool, Int, IOSurface, UInt32, UInt8

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
Cohesion: 0.20
Nodes (12): SPEC_CHUM_AGENT_URL remote mode, spec-chum-debugging skill, agent_server crate, debug_cli crate, spec-chum-agent, spec-chum-debug, Agent Debug HTTP API, Loopback HTTP 127.0.0.1:17384 (+4 more)

### Community 94 - "KempstonMouse"
Cohesion: 0.22
Nodes (6): buttons_active_low(), delta_wraps_axes(), KempstonMouse, port_reads(), Option, Self

### Community 95 - "rzx.rs"
Cohesion: 0.25
Nodes (11): apply_input_byte(), apply_matrix_and_kempston(), minimal_rzx(), parse_input_frames(), FnMut, Path, Result, Self (+3 more)

### Community 96 - ".onLivingRoomDisplayTick"
Cohesion: 0.15
Nodes (5): InputLatencyProbe, UInt64, CFAbsoluteTime, CFTimeInterval, Int32

### Community 97 - "agent_embed.rs"
Cohesion: 0.05
Nodes (54): AtomicU32, CGImage, EmbeddedServer, Arc, Option, Result, String, spawn() (+46 more)

### Community 98 - "Bus128"
Cohesion: 0.18
Nodes (5): Bus128, out_7ffd_records_display_screen_events(), out_7ffd_screen_switch_spills_into_next_frame(), page_7ffd(), Vec

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

### Community 104 - "MachineConfigEditorView"
Cohesion: 0.16
Nodes (13): GlassBarBackground, View, MachineConfigEditorView, .body, .hardwareCompat, Bool, String, UserMachineConfig (+5 more)

### Community 105 - "z80test.rs"
Cohesion: 0.22
Nodes (17): assert_z80test_passed(), code_block(), fixture_dir(), rom48_path(), Duration, Path, PathBuf, Result (+9 more)

### Community 106 - "HostSession"
Cohesion: 0.07
Nodes (10): HostAccess, Deref, DerefMut, MutexGuard, HostSession, open_fixture_tap_progress_and_audio_pcm(), Into, Machine (+2 more)

### Community 107 - "attach_crt_to_television"
Cohesion: 0.25
Nodes (11): animate_crt_params(), attach_crt_to_television(), Entity, MeshMaterial3d, Option, Query, Res, Time (+3 more)

### Community 108 - "NSEvent"
Cohesion: 0.10
Nodes (7): KempstonMouseTracking, Bool, Int, NSEvent, NSView, NSEvent, NSEvent

### Community 109 - "host_api crate"
Cohesion: 0.20
Nodes (10): ./scripts/check.sh, host_api crate, living_room crate, C ABI FFI-only policy, Guest framebuffer PNG export, Bevy 3D CRT living-room host, spec-chum-room, SCLD 512x192 hi-res modes (+2 more)

### Community 110 - "Full slow test suite"
Cohesion: 0.31
Nodes (11): ./scripts/run_slow_tests.sh, TDD expectations, Full slow test suite, CI z80doc job, ./scripts/run_system_tests.sh, Fuse Z80 test vectors, minfo.tap, Third-party system tests (+3 more)

### Community 111 - "living_room/src/lib.rs"
Cohesion: 0.27
Nodes (7): AssetPlugin, asset_plugin(), living_room_app(), resolve_asset_root(), App, PathBuf, main()

### Community 112 - "control_plane shared backend"
Cohesion: 0.50
Nodes (4): control_plane crate, control_plane shared backend, HostSession, Single source of truth backend

### Community 113 - ".body"
Cohesion: 0.08
Nodes (18): App, ContentView, .livingRoomToolbar, .statusFooter, .body, RomSetupSlotRow, .body, RomSetupView (+10 more)

### Community 114 - "Kempston"
Cohesion: 0.28
Nodes (3): bits_active_high(), Kempston, Self

### Community 115 - "CodingKeys"
Cohesion: 0.12
Nodes (17): CodingKeys, attachBeta, attachDivmmc, attachInterface1, attachMultiface, ayStereo, base, customRomPath (+9 more)

### Community 116 - "error.rs"
Cohesion: 0.10
Nodes (17): encode_framebuffer_png(), parse_model_slug(), ApiResult, Vec, HostViewState, HostWindowCapture, new_shared_host_view(), ApiResult (+9 more)

### Community 117 - "file_dialog.rs"
Cohesion: 0.29
Nodes (6): FileDialogPlugin, OpenMediaDialog, App, Plugin, ResMut, run_open_dialog()

### Community 118 - "check_pr_reviews.sh"
Cohesion: 0.43
Nodes (6): append_bot_thread_from_comments(), apply_waiver_or_fail(), expect(), check_pr_reviews.sh script, classify_coderabbit_head_status(), pr_review_cr_classify.sh script

### Community 119 - "Spec Chum"
Cohesion: 0.17
Nodes (16): ./scripts/fetch_roms.sh, Spec Chum, Dual-clock embed architecture, SpecChumMac SwiftUI shell, Multiface 1, Release process, Amstrad Lawson redistribution grant, ROM fetch policy (+8 more)

### Community 121 - "theme.rs"
Cohesion: 0.43
Nodes (5): apply(), apply_does_not_panic_on_default_context(), clear_color(), panel_fill_is_opaque(), Context

### Community 122 - "room_perf.rs"
Cohesion: 0.43
Nodes (6): main(), percentile_ms(), Duration, Vec, summarize(), varying_frame()

### Community 123 - "import_iosurface_texture"
Cohesion: 0.38
Nodes (6): import_iosurface_texture(), c_void, RenderDevice, Result, String, Texture

### Community 124 - "custom_loader_tap"
Cohesion: 0.71
Nodes (6): checksum(), custom_loader_tap(), main(), make_code_tap(), CODE that `CALL 0556` (ROM LD-BYTES) for a following flag-0xC8 block. Models…, tap_block()

### Community 125 - "CodeRabbit merge gate"
Cohesion: 0.33
Nodes (6): ./scripts/check_pr_reviews.sh, auto_review.enabled false, coderabbit-review label, CodeRabbit on-demand reviews, CodeRabbit merge gate, Bot review threads gate

### Community 126 - "tick_emulator"
Cohesion: 0.27
Nodes (11): CameraLocked, host_hotkeys(), Assets, ButtonInput, Image, KeyCode, Option, Res (+3 more)

### Community 127 - "disasm.rs"
Cohesion: 0.24
Nodes (24): abs_ed(), abs_mem(), alu_imm(), Disasm, disasm_cb(), disasm_ddcb(), disasm_ed(), disasm_index() (+16 more)

### Community 128 - "fb_scale.rs"
Cohesion: 0.40
Nodes (4): blit_to_crt(), dims_from_rgba_len(), Option, scale_hires_paper_to_crt()

### Community 129 - "PrefModel"
Cohesion: 0.29
Nodes (3): pref_model_slug(), PrefModel, Model

### Community 130 - "fetch_roms.sh"
Cohesion: 0.67
Nodes (5): checkout_sparse_repo(), copy_dir_roms(), copy_rom(), count_managed_roms(), fetch_roms.sh script

### Community 131 - "app crate"
Cohesion: 0.50
Nodes (4): app crate, egui/eframe primary host, Spec Chum.app release bundle, Release workflow

### Community 132 - "Flash-load at LD-BYTES 0x056C"
Cohesion: 0.40
Nodes (5): Flash-load convenience exception, Flash-load at LD-BYTES 0x056C, type-load subcommand, attr_mark.tap, custom_loader.tap

### Community 133 - "Self"
Cohesion: 0.25
Nodes (4): PrefAyStereo, AyStereoMode, JoystickMode, Self

### Community 134 - "fetch_system_tests.sh"
Cohesion: 0.80
Nodes (4): fetch(), fetch_system_tests.sh script, sha256_of(), verify_sha()

### Community 135 - "room_perf_matrix.sh"
Cohesion: 0.50
Nodes (4): run_one(), RUST_LOG, room_perf_matrix.sh script, SPEC_CHUM_ROOM_PERF_SOFT

### Community 136 - ".write"
Cohesion: 0.24
Nodes (6): ram16k_maps_only_low_ram(), rom_ram_map(), timex_2068_dock_ram_chunk_writable(), timex_2068_dock_spectrum_rom_pages_via_hsr(), timex_2068_exrom_pages_chunk0(), timex_2068_home_bank_spectrum_rom_replace()

### Community 137 - "FramebufferMeta"
Cohesion: 0.28
Nodes (5): FramebufferMeta, model_slug(), Option, Self, String

### Community 138 - "CrtPlugin"
Cohesion: 0.50
Nodes (3): CrtPlugin, App, Plugin

### Community 139 - "HybridPlugin"
Cohesion: 0.50
Nodes (3): HybridPlugin, App, Plugin

### Community 140 - "PresentBlitPlugin"
Cohesion: 0.50
Nodes (3): PresentBlitPlugin, App, Plugin

### Community 141 - "parse_model"
Cohesion: 0.38
Nodes (6): Cli, main(), parse_model(), Option, Result, String

### Community 142 - "UiOverlayPlugin"
Cohesion: 0.50
Nodes (3): App, Plugin, UiOverlayPlugin

### Community 143 - "check_crates.sh"
Cohesion: 0.67
Nodes (3): infer_crates(), RUSTFLAGS, check_crates.sh script

### Community 172 - "mid_line_border_128_uses_228_pitch"
Cohesion: 0.40
Nodes (3): mid_line_border_128_uses_228_pitch(), mid_line_border_change_splits_scanline(), render_smoke()

### Community 173 - "Vec"
Cohesion: 0.22
Nodes (5): map_host_model_error(), From, Vec, WatchesResponse, WatchSpec

## Knowledge Gaps
- **147 isolated node(s):** `PackageDescription`, `Notification.Name`, `.isMenuTracking`, `.body`, `GameController` (+142 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 714 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **28 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Machine` connect `Machine` to `.add_t`, `Bus128`, `.step_once`, `Ula48`, `Bus48`, `machine/src/lib.rs`, `.new_48k`, `joystick.rs`, `Ay8912`, `Kempston`, `custom_loader_matrix_models_instant_and_ear`, `interface1_opcode_fetch_pages_shadow_rom`, `Debugger`, `.cpu`, `BusPlus3`, `KempstonMouse`?**
  _High betweenness centrality (0.214) - this node is a cross-community bridge._
- **Why does `HostBridge` connect `HostBridge` to `Model`, `.onLivingRoomDisplayTick`, `SpectrumDisplayView`, `LivingRoomNSView`, `agent_embed.rs`, `PrefModelSlug`, `MachineConfigEditorView`, `RomSetupSlot`, `NSEvent`, `.body`, `SpectrumNSView`, `Bool`, `.body`, `.takeLastError`, `TapeAudioPlayer`?**
  _High betweenness centrality (0.196) - this node is a cross-community bridge._
- **Why does `HostSession` connect `HostSession` to `.new`, `host_api/src/ffi.rs`, `agent_embed.rs`, `.recompose_input`, `AgentClient`, `HostError`, `FramebufferMeta`, `.new`, `.open_tape`, `joystick.rs`, `ControlPlane`, `.with_host_mut`, `service.rs`, `.load_rom_bytes_with_overrides`, `error.rs`, `session.rs`, `ModelId`, `EmulatorHost`?**
  _High betweenness centrality (0.152) - this node is a cross-community bridge._
- **Are the 4 inferred relationships involving `HostBridge` (e.g. with `.livingRoomToolbar` and `TapeAudioPlayer`) actually correct?**
  _`HostBridge` has 4 INFERRED edges - model-reasoned connections that need verification._
- **What connects `PackageDescription`, `Notification.Name`, `.isMenuTracking` to the rest of the system?**
  _147 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `routes.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0644910644910645 - nodes in this community are weakly interconnected._
- **Should `host_api/src/ffi.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.07866795366795366 - nodes in this community are weakly interconnected._