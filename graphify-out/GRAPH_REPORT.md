# Graph Report - g5c4  (2026-09-02)

## Corpus Check
- 164 files · ~196,402 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 3870 nodes · 10571 edges · 173 communities (140 shown, 26 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 262 edges (avg confidence: 0.83)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Agent Debug API
- Host API FFI
- Formats
- Trace
- Machine Core
- macOS HostBridge
- Debug
- Host Session
- Tape / Machine
- Bus
- Ula
- Machine
- Machine
- Control Plane
- Bus
- Tape
- Bus
- Macos App
- Host / App
- Machine Wiring
- Living Room Bevy
- Host
- Machine / Tape
- Living
- Machine
- Macos App
- Living
- Bus
- Macos App
- Macos App
- Host
- Living
- Macos App
- Bus
- App
- Macos App
- Living
- Formats
- Host
- Macos App
- App
- Control
- Z80 CPU Core
- Macos App
- App
- Bus
- Control
- Macos App
- Macos App
- Macos App
- Bus
- Machine
- Living
- Z80
- Living
- Machine
- Host
- Machine
- Bus
- Living
- Z80
- Formats
- Living
- Living
- Z80
- Macos App
- App
- Living
- Z80
- Host / Agent
- Bus
- Formats
- Machine
- Z80
- Bus
- Host
- Machine
- Control
- Formats
- Living
- Living
- Machine
- Macos App
- Living
- Living
- Macos App
- Macos App
- Living
- Macos App
- Bus
- Z80
- Z80
- Docs
- Macos App
- Bus
- Formats
- Host
- Agent
- Bus
- Living
- Living
- Living
- Misc
- Docs
- App
- Host
- Host
- Living
- Living
- Docs
- Slow Tests Sh
- Living
- Control
- Host
- Bus
- Bus
- Control
- Living
- Scripts
- Docs
- App / Host
- App
- Living
- Living
- Scripts
- Pr Reviews Sh
- App
- App
- Living
- Machine
- Scripts
- Docs
- Docs
- Host
- Scripts
- Scripts
- Macos App
- Living
- Living
- Living
- Living
- Living
- Scripts
- Scripts
- Skills Spec Chum
- Scripts
- Scripts
- Scripts
- Scripts
- Scripts
- Scripts
- Trace Crate
- Misc
- Room Check Living
- Native Build Macos
- Multiface 1
- Z80Test Sh 
- Macos App
- Slow Tests
- System Tests
- Living Room Assets
- Misc
- Crates Sh
- Misc
- Plus3 Plus3Dos Smokes
- Tape Tape Fixtures

## God Nodes (most connected - your core abstractions)
1. `HostBridge` - 205 edges
2. `HostSession` - 131 edges
3. `Machine` - 112 edges
4. `ControlPlane` - 95 edges
5. `AppState` - 75 edges
6. `session_mut()` - 64 edges
7. `HostError` - 63 edges
8. `Cpu` - 63 edges
9. `LivingRoomNSView` - 59 edges
10. `set_last_error()` - 54 edges

## Surprising Connections (you probably didn't know these)
- `Spec Chum` --references--> `Release process`  [EXTRACTED]
  README.md → docs/RELEASE.md
- `ulatest3.tap` --references--> `ula crate`  [INFERRED]
  tests/fixtures/system/README.md → AGENTS.md
- `synthetic_plus3_boot_marker` --references--> `formats crate`  [INFERRED]
  tests/fixtures/plus3/README.md → AGENTS.md
- `Release workflow` --references--> `debug_cli crate`  [EXTRACTED]
  .github/workflows/release.yml → AGENTS.md
- `control_plane crate` --references--> `Agent Debug HTTP API`  [EXTRACTED]
  AGENTS.md → docs/AGENT_DEBUG_API.md

## Import Cycles
- 2-file cycle: `crates/z80/src/cpu.rs -> crates/z80/src/opcodes.rs -> crates/z80/src/cpu.rs`
- 2-file cycle: `crates/formats/src/lib.rs -> crates/formats/src/mdr.rs -> crates/formats/src/lib.rs`
- 2-file cycle: `crates/formats/src/lib.rs -> crates/formats/src/trd.rs -> crates/formats/src/lib.rs`
- 2-file cycle: `crates/formats/src/dsk.rs -> crates/formats/src/lib.rs -> crates/formats/src/dsk.rs`
- 2-file cycle: `crates/formats/src/dck.rs -> crates/formats/src/lib.rs -> crates/formats/src/dck.rs`
- 2-file cycle: `crates/formats/src/lib.rs -> crates/formats/src/rzx.rs -> crates/formats/src/lib.rs`
- 3-file cycle: `crates/formats/src/dsk.rs -> crates/formats/src/lib.rs -> crates/formats/src/fdc.rs -> crates/formats/src/dsk.rs`

## Hyperedges (group relationships)
- **Emulator core crate stack** — agents_z80_crate, agents_bus_crate, agents_ula_crate, agents_tape_crate, agents_formats_crate, agents_machine_crate [EXTRACTED 1.00]
- **Host UI surfaces** — agents_app_crate, docs_macos_native_specchummac, docs_living_room_bevy_crt_host, agents_host_api_crate [EXTRACTED 1.00]
- **Agent debug control plane** — docs_agent_debug_api_control_plane, agents_agent_server_crate, agents_debug_cli_crate, docs_agent_debug_api_host_session, docs_agent_debug_api_loopback_http [EXTRACTED 1.00]

## Communities (173 total, 26 thin omitted)

### Community 0 - "Agent Debug API"
Cohesion: 0.06
Nodes (143): add_breakpoint(), add_port_watch(), add_watch(), api_error(), apply_config(), AppState, attach_divmmc(), attach_interface1() (+135 more)

### Community 1 - "Host API FFI"
Cohesion: 0.06
Nodes (122): break_reason_code(), clear_last_error(), ffi_bad_model_returns_null(), ffi_create_destroy_and_run(), ffi_debug_dump_json_and_peek_null(), ffi_joystick_mode_rejects_truncated_overflow(), ffi_mouse_delta_and_buttons_smoke(), heap_cstring() (+114 more)

### Community 2 - "Formats"
Cohesion: 0.05
Nodes (53): cpm_dir_entry(), DskImage, find_id_matches_r_without_chrn_c(), multi_sector_dsk_lookup(), parse_and_read_sector(), parse_track(), plus3_basic_poke_marker(), plus3_cpm_chs() (+45 more)

### Community 3 - "Trace"
Cohesion: 0.06
Nodes (59): AsRef, BitOr, BitOrAssign, BufWriter, append_flushes_events_to_trace_file(), AppendSink, categories(), Category (+51 more)

### Community 4 - "Machine Core"
Cohesion: 0.09
Nodes (75): expected_main_rom_bytes(), exrom_available(), exrom_available_in(), exrom_candidates(), install_rom_slot(), install_rom_slot_copies_to_expected_path(), install_rom_slot_validates_size(), main_rom_available() (+67 more)

### Community 5 - "macOS HostBridge"
Cohesion: 0.05
Nodes (29): HostBridge, .hasTimexDock, .isCustomConfigActive, .joystickMode, .kempstonMouse, .livingRoomMode, .machineDisplayTitle, .needsRomSetup (+21 more)

### Community 6 - "Debug"
Cohesion: 0.08
Nodes (32): Agent, AgentClient, AuthRequest, Option, Result, Self, String, Value (+24 more)

### Community 7 - "Host Session"
Cohesion: 0.07
Nodes (8): HostError, HostSession, Error, Into, Path, Result, String, BreakReason

### Community 8 - "Tape / Machine"
Cohesion: 0.07
Nodes (44): game_running(), load_deathchase(), plus2a_48basic_instant_still_runs_deathchase(), plus2a_menu_loader_ear_runs_deathchase(), plus2a_menu_loader_instant_runs_deathchase(), rom_plus2a(), Machine, Option (+36 more)

### Community 9 - "Bus"
Cohesion: 0.07
Nodes (26): decode_port(), Drive, If1Port, Interface1, Interface1RomError, mdr_roundtrip_via_if1(), motor_select_and_sector_stream_read(), motor_select_and_sector_stream_write() (+18 more)

### Community 10 - "Ula"
Cohesion: 0.07
Nodes (32): contention_delay(), contention_delay_128(), contention_delay_48(), contention_delay_params(), floating_bus_byte(), floating_bus_byte_128(), floating_bus_byte_48(), floating_bus_params() (+24 more)

### Community 11 - "Machine"
Cohesion: 0.10
Nodes (57): assert_contended_nop_pattern(), assert_pattern(), assert_screen_has(), azesmbog_loads_and_paints(), azesmbog_ula128_timing_paints(), azesmbog_ula128e_plus3_paints(), azesmbog_ula48_simple_paints(), bitmap_addr() (+49 more)

### Community 12 - "Machine"
Cohesion: 0.15
Nodes (37): apply_sna48_sets_pc_ram_and_border(), apply_z80_snapshot48_sets_pc_ram_and_border(), attr_mark_code_ok(), attr_mark_ear_load_quotes_code_succeeds_at_speed_10(), attr_mark_experience_load_succeeds(), attr_mark_fixture_flash_loads_code_bytes(), attr_mark_load_path_dumps_trace_on_failure(), attr_mark_load_path_must_succeed() (+29 more)

### Community 13 - "Control Plane"
Cohesion: 0.11
Nodes (5): ControlPlane, ApiResult, Mutex, Path, UserMachineConfig

### Community 14 - "Bus"
Cohesion: 0.12
Nodes (25): BetaDisk, disk_not_ready_without_image(), drive_b_is_not_ready(), force_interrupt_clears_drq(), m1_48k_pages_at_3c00_128k_does_not(), m1_does_not_unpage_while_executing_trdos_rom(), m1_pages_rom_in_at_3d00_and_out_at_4000(), multiple_read_walks_sectors() (+17 more)

### Community 15 - "Tape"
Cohesion: 0.10
Nodes (30): active_pulse_counters_are_block_relative(), advance_clears_high_ear_on_exact_final_pulse_end(), advance_clears_playing_on_exact_final_pulse_boundary(), append_pure_data(), append_standard_block(), append_turbo_block(), empty_tzx_reports_zero_blocks(), minimal_tzx_standard() (+22 more)

### Community 16 - "Bus"
Cohesion: 0.11
Nodes (23): automap_entry_and_exit_with_eeprom(), automap_ignored_without_eeprom_or_mapram(), control_port_conmem_shows_ram_page(), DivMmc, eeprom_accepts_larger_image_prefix(), mapram_is_sticky_across_control_writes(), mapram_uses_page_3_in_lower_8k(), Default (+15 more)

### Community 17 - "Macos App"
Cohesion: 0.09
Nodes (16): SpectrumNSView, .acceptsFirstResponder, .canBecomeKeyView, .focusRingMaskBounds, .isFlipped, Any, Bool, Context (+8 more)

### Community 18 - "Host / App"
Cohesion: 0.09
Nodes (36): chord_for(), letter_digit(), modifier_keys(), mods_shift(), punct_chord(), quote_alone_is_symbol_7(), quote_shift_is_symbol_p(), Key (+28 more)

### Community 19 - "Machine Wiring"
Cohesion: 0.09
Nodes (11): TimexDockError, custom_loader_ok(), Machine, plus3_loader_dos_boot_runs_titled_marker(), plus3_loader_load_disk_runs_basic_marker(), plus3_loader_talks_to_fdc_on_data_disk(), reset_keeps_plus3_disk_inserted(), rom_plus3_only() (+3 more)

### Community 20 - "Living Room Bevy"
Cohesion: 0.09
Nodes (37): Bloom, anim_eases_toward_target(), apply_zoom_camera(), CameraZoom, CrtLookBlend, IntroSkipRequest, nudge_clamps(), pose_at_zoom() (+29 more)

### Community 21 - "Host"
Cohesion: 0.15
Nodes (34): border_toggle_resizes_framebuffer(), cursor_left_via_joystick_applies_caps_five(), dims(), joystick_kempston_mask_reaches_port(), kempston_arrow_left_does_not_pollute_matrix(), kempston_mouse_ports_after_synthetic_deltas(), load_dsk_rejects_non_plus3(), load_rom_and_run_frame_writes_pixels() (+26 more)

### Community 22 - "Machine / Tape"
Cohesion: 0.10
Nodes (16): advance_frame_t(), mem_port_watch(), multiface_nmi_executes_attached_rom(), peek_opcode(), reg_snap(), rom_timex_ts2068(), Option, TapeDeck (+8 more)

### Community 23 - "Living"
Cohesion: 0.16
Nodes (41): Camera, Children, LivingRoomCamera, apply_hybrid_display(), bind_present_target(), ensure_plate_image(), hybrid_state_machine(), HybridBakeCamera (+33 more)

### Community 24 - "Machine"
Cohesion: 0.07
Nodes (33): apply_snapshot128_plus3_applies_1ffd(), apply_snapshot128_z80_pages_and_7ffd(), attr_mark_type_load_128k_flash(), beta_trdos_rom_loop_reads_trd_sector_into_ram(), cpu_step_appears_in_trace(), custom_loader_tap(), FrameAudio, inspect_128k_paging() (+25 more)

### Community 25 - "Macos App"
Cohesion: 0.08
Nodes (25): activateSpecChum(), AppDelegate, Notification.Name, ContentView, .body, .flatSpectrumChrome, .livingRoomChrome, FocusSpectrumView (+17 more)

### Community 26 - "Living"
Cohesion: 0.09
Nodes (36): Buffer, despawn_image_copiers(), drain_copied_frames(), image_copy_driver(), image_copy_extract(), ImageCopier, ImageCopiers, ImageCopyPlugin (+28 more)

### Community 27 - "Bus"
Cohesion: 0.11
Nodes (18): BusPlus3, contended_banks_are_4_through_7(), fdc_motor_bit_on_1ffd_affects_st3(), fdc_read_data_protocol_via_ports(), is_contended_bank_plus3(), lock_blocks_both_ports(), no_floating_bus(), out_7ffd_address_does_not_hit_1ffd() (+10 more)

### Community 29 - "Macos App"
Cohesion: 0.13
Nodes (21): AudioCaptureFile, AudioLog, Stats, Bool, Double, Float, Int, String (+13 more)

### Community 30 - "Host"
Cohesion: 0.14
Nodes (31): AppliedConfig, apply_builtin_rom_when_no_override(), apply_rejects_bad_custom_rom_size(), apply_user_config(), build_machine(), forty8k_keeps_divmmc_if1_beta(), hardware_compat(), HardwareCompat (+23 more)

### Community 31 - "Living"
Cohesion: 0.18
Nodes (36): catch_const_u8(), catch_int(), catch_ptr(), catch_uint(), clear_last_error(), room_mut(), RoomHandle, c_char (+28 more)

### Community 32 - "Macos App"
Cohesion: 0.06
Nodes (28): JoystickMode, cursor, .id, kempston, sinclairLeft, sinclairRight, .title, Model (+20 more)

### Community 33 - "Bus"
Cohesion: 0.14
Nodes (15): acb_vs_abc_pan_differs(), acb_vs_abc_swap_b_and_c_pans(), Ay8912, ay_channel_b_only(), envelope_level(), envelope_write_restarts(), mixer_mute_silence(), mono_stereo_matches_sample_mono() (+7 more)

### Community 34 - "App"
Cohesion: 0.17
Nodes (10): EmulatorSession, load_snapshot48_switches_from_128k(), load_snapshot48_switches_from_plus3(), play_tape_advances_ear_on_fixture(), plus3_type_load_code_flash_loads_attr_mark(), Modifiers, session_loads_tap_fixture_headless(), synthetic_sna48_bytes() (+2 more)

### Community 35 - "Macos App"
Cohesion: 0.12
Nodes (14): LivingRoomNSView, .acceptsFirstResponder, .canBecomeKeyView, .focusRingMaskBounds, .host, .isFlipped, Any, Bool (+6 more)

### Community 36 - "Living"
Cohesion: 0.11
Nodes (26): AtomicUsize, audio_capture_enabled(), AudioMuted, AudioOut, AudioPlugin, AudioStream, DcBlock, fill_output() (+18 more)

### Community 37 - "Formats"
Cohesion: 0.13
Nodes (28): parse_sna128_regs_banks_pc(), parse_sna128_when_paged_is_bank5(), parse_z80_header(), parse_z80_rejects_undersized_extended_header(), parse_z80_v1_compressed_regs_and_ram(), parse_z80_v1_uncompressed_regs_and_ram(), parse_z80_v2_128_banks_and_7ffd(), parse_z80_v2_pages_land_at_48k_addresses() (+20 more)

### Community 38 - "Host"
Cohesion: 0.14
Nodes (19): corrupt_file_falls_back_to_defaults(), custom_configs_round_trip(), load_prefs(), load_prefs_unlocked(), missing_file_falls_back_to_defaults(), model_rom_paths_round_trip(), recent_files_most_recent_first_deduped(), round_trip_preserves_fields() (+11 more)

### Community 39 - "Macos App"
Cohesion: 0.09
Nodes (27): PrefAyStereoSlug, abc, acb, mono, PrefJoystickSlug, cursor, .hostMode, kempston (+19 more)

### Community 40 - "App"
Cohesion: 0.15
Nodes (16): arrow_left_maps_joystick_kempston_and_cursor_mode(), debug_window_smoke_headless(), egui_menu_smoke_without_window(), emulator_session_uses_host_session(), gui_and_control_plane_share_live_session(), HostSlot, KeyScript, physical_num1_survives_sinclair_left_joystick_clear() (+8 more)

### Community 41 - "Control"
Cohesion: 0.12
Nodes (17): apply_prefs_to_session(), continue_and_eject_require_machine(), health_and_inspect_after_rom_load(), mouse_requires_kempston_pref_then_accepts_input(), prefs_apply_after_rom_load(), prefs_patch_round_trip(), rom48(), Arc (+9 more)

### Community 42 - "Z80 CPU Core"
Cohesion: 0.18
Nodes (31): add16(), adc16(), block_cp(), block_in(), block_ld(), block_out(), condition(), daa() (+23 more)

### Community 43 - "Macos App"
Cohesion: 0.12
Nodes (22): AppKit, JsonRoot, JsonSlot, RomSetupCodec, RomSetupPayload, RomSetupSlot, .sizeHint, .statusColor (+14 more)

### Community 44 - "App"
Cohesion: 0.09
Nodes (22): BeeperState, App, Arc, BTreeMap, Debug, Default, Formatter, Instant (+14 more)

### Community 45 - "Bus"
Cohesion: 0.12
Nodes (15): button_pages_on_nmi_vector(), in_9f_pages_in_in_1f_pages_out(), load_rom_size_check(), Multiface1, multiface1_port_match(), out_1f_clears_nmi_pending_without_unpaging(), out_3f_is_not_mf1_decode(), reset_clears_paging_keeps_ram() (+7 more)

### Community 46 - "Control"
Cohesion: 0.11
Nodes (16): format_break_reason(), last_error_records_failures(), LastBreakResponse, LastErrorRecord, MemoryMapResponse, MemoryRegion, PagingSnapshot, PrefsPatch (+8 more)

### Community 47 - "Macos App"
Cohesion: 0.12
Nodes (6): .livingRoomToolbar, .experienceLoad, .instantLoad, .model, .tapeSpeed, UserMachineConfig

### Community 48 - "Macos App"
Cohesion: 0.11
Nodes (20): .statusFooter, GlassBarBackground, View, MachineConfigEditorView, .body, .hardwareCompat, Bool, String (+12 more)

### Community 49 - "Macos App"
Cohesion: 0.11
Nodes (7): App, DebugInspectorView, .body, SpecChumMacApp, .body, Int32, Scene

### Community 50 - "Bus"
Cohesion: 0.16
Nodes (22): beta_ports_when_trdos_paged_via_bus48(), beta_trdos_rom_overlays_when_paged(), bus128_m1_pages_trdos_at_3d00_not_3c00(), contend_128_differs_from_48_at_paper_start(), interface1_shadow_rom_mirror_via_bus48(), kempston_mouse_ports_after_delta_and_buttons(), kempston_port_1f(), kempston_port_1f_untouched_when_beta_attached_but_not_paged() (+14 more)

### Community 51 - "Machine"
Cohesion: 0.14
Nodes (11): ay_frame_audio_nonzero_when_tone_programmed(), emit_contend_sampled(), MemIo128, MemIo48, MemIoPlus3, multiface_in_pages_out_and_back_for_return(), plus2a_stack_repair_ignores_coincidental_0038_marker(), timex_scld_ext_colour_render_uses_alt_attrs() (+3 more)

### Community 52 - "Living"
Cohesion: 0.13
Nodes (27): BackgroundColor, Changed, ChildSpawnerCommands, CameraIntro, chrome_button(), chrome_buttons(), ChromeAction, host_cmd_shortcuts() (+19 more)

### Community 53 - "Z80"
Cohesion: 0.24
Nodes (24): abs_ed(), abs_mem(), alu_imm(), Disasm, disasm_cb(), disasm_ddcb(), disasm_ed(), disasm_index() (+16 more)

### Community 54 - "Living"
Cohesion: 0.11
Nodes (8): HeadlessRoom, rebuild_headless_render_target(), Debug, Formatter, Result, Self, String, Write

### Community 55 - "Machine"
Cohesion: 0.12
Nodes (8): Debugger, Cell, Default, Option, Self, Vec, Watch, WatchHook

### Community 56 - "Host"
Cohesion: 0.19
Nodes (23): canonical_persist_path(), install_model_rom(), model_rom_available(), model_rom_paths_snapshot(), pentagon_rom_setup_has_user_slots(), persisted_path_wins_over_missing_workspace(), rom_setup_json(), rom_setup_json_serializes() (+15 more)

### Community 57 - "Machine"
Cohesion: 0.24
Nodes (11): attr_mark_load_matrix_models_and_speeds(), attr_mark_type_load_plus3_flash(), boggit_side1_matrix_when_present(), custom_loader_matrix_models_instant_and_ear(), MachineBuildError, plus3_boots_and_1ffd_special_maps(), rom_plus3(), rom_timex_tc2048() (+3 more)

### Community 58 - "Bus"
Cohesion: 0.12
Nodes (8): altmembank_and_chunk_bits(), port_f4_latches(), port_ff_read_returns_last_write(), Option, Self, screen_mode_and_int_disable_from_port_ff(), TimexScld, TimexScreenMode

### Community 59 - "Living"
Cohesion: 0.12
Nodes (14): EmulatorHost, HostPlugin, model_label(), App, Debug, Duration, Formatter, PathBuf (+6 more)

### Community 60 - "Z80"
Cohesion: 0.11
Nodes (7): pairs_round_trip(), r_preserves_bit7(), Registers, Display, Formatter, Result, Self

### Community 61 - "Formats"
Cohesion: 0.18
Nodes (15): decode_z80_page(), decode_z80_v1(), FormatError, load_z80_v2_pages_128(), load_z80_v2_pages_48(), regs_from_z80_header(), Display, Error (+7 more)

### Community 62 - "Living"
Cohesion: 0.11
Nodes (21): CrtPhosphor, CrtFillLight, FrameGlow, GlowDriven, GlowPlugin, IncandescentLamp, red_border_dominates_glow(), App (+13 more)

### Community 63 - "Living"
Cohesion: 0.11
Nodes (20): blit_to_present(), extract_present_target(), ExtractedPresent, PresentTarget, Arc, Commands, Debug, Extract (+12 more)

### Community 64 - "Z80"
Cohesion: 0.23
Nodes (5): Cpu, Option, Vec, I, M

### Community 65 - "Macos App"
Cohesion: 0.14
Nodes (5): KempstonMouseTracking, Bool, Int, NSEvent, NSEvent

### Community 66 - "App"
Cohesion: 0.18
Nodes (4): Context, Path, UserMachineConfig, Frame

### Community 67 - "Living"
Cohesion: 0.13
Nodes (16): CameraPlugin, clamp01(), distance_for_crt_fill(), distance_matches_fill(), ease_in_out_cubic(), ease_out_cubic(), easing_endpoints(), lerp_eye_pullback_rise() (+8 more)

### Community 68 - "Z80"
Cohesion: 0.21
Nodes (21): FuseEvent, Expected, fixtures_dir(), format_fuse_event(), fuse_all_vectors(), fuse_disasm_window(), fuse_mismatch_includes_disasm_at_start_pc(), fuse_smoke_nop() (+13 more)

### Community 69 - "Host / Agent"
Cohesion: 0.14
Nodes (10): Cli, main(), parse_model(), Option, Result, String, load_128_or_plus3_rom(), ModelId (+2 more)

### Community 70 - "Bus"
Cohesion: 0.18
Nodes (4): Bus128, emit_floating_sampled(), Option, Vec

### Community 71 - "Formats"
Cohesion: 0.19
Nodes (12): DckBank, DckBankId, DckChunkAccess, DckImage, parse_home_replace_and_empty_ram(), parse_spectrum_dock_header(), reject_truncated_pages(), reject_unknown_bank() (+4 more)

### Community 72 - "Machine"
Cohesion: 0.14
Nodes (14): Inspect, Machine, opt_u8(), Paging, Display, FmtResult, Formatter, Model (+6 more)

### Community 73 - "Z80"
Cohesion: 0.13
Nodes (8): FlatMem, Io, Memory, NullIo, Box, Default, Self, FuseBus

### Community 74 - "Bus"
Cohesion: 0.13
Nodes (4): Bus48, Keyboard, Default, Self

### Community 75 - "Host"
Cohesion: 0.18
Nodes (10): model_rom_path_key(), pref_model_slug_matches_json_snake_case(), BTreeMap, Default, Option, String, UserMachineConfig, Vec (+2 more)

### Community 76 - "Machine"
Cohesion: 0.20
Nodes (13): apply_joystick(), clear_joystick_matrix(), cursor_uses_caps_and_5678(), JoystickMode, JoystickState, kempston_mask_roundtrip(), kempston_mode_sets_port_bits(), release_clears_previous_matrix() (+5 more)

### Community 77 - "Control"
Cohesion: 0.14
Nodes (8): model_slug(), Self, String, HardwareStatusResponse, HealthResponse, FnOnce, R, StatusResponse

### Community 78 - "Formats"
Cohesion: 0.23
Nodes (9): parse_and_read_sector(), Option, Path, Result, Self, Vec, synthetic_trd(), TrdImage (+1 more)

### Community 79 - "Living"
Cohesion: 0.16
Nodes (11): aperture_debug_enabled(), ApertureDebugMarker, bottom_adjust_keeps_top_edge(), bright_debug_enabled(), crt_phosphor_local(), crt_screen_world_center(), env_flag(), hide_crt_enabled() (+3 more)

### Community 80 - "Living"
Cohesion: 0.18
Nodes (17): bind_hybrid_headless_targets(), create_headless_render_image(), HeadlessRenderTargetHandle, HeadlessSize, Assets, c_void, Commands, Entity (+9 more)

### Community 81 - "Machine"
Cohesion: 0.22
Nodes (17): assert_z80test_passed(), code_block(), fixture_dir(), rom48_path(), Duration, Path, PathBuf, Result (+9 more)

### Community 82 - "Macos App"
Cohesion: 0.12
Nodes (17): CodingKeys, attachBeta, attachDivmmc, attachInterface1, attachMultiface, ayStereo, base, customRomPath (+9 more)

### Community 83 - "Living"
Cohesion: 0.24
Nodes (16): AssetServer, pbr_material(), Assets, Commands, Handle, Mesh, Option, Res (+8 more)

### Community 84 - "Living"
Cohesion: 0.17
Nodes (16): bulge_mesh_has_expected_vertex_count(), bulging_screen_mesh(), CrtPhosphorMaterial, CrtScreenTexture, CrtSpawnKit, Assets, Commands, Handle (+8 more)

### Community 85 - "Macos App"
Cohesion: 0.17
Nodes (4): InputLatencyProbe, UInt64, CFAbsoluteTime, CFTimeInterval

### Community 86 - "Macos App"
Cohesion: 0.37
Nodes (6): SpectrumKeymap, Bool, NSEvent, Set, UInt16, UInt32

### Community 87 - "Living"
Cohesion: 0.13
Nodes (13): ExternalFramebuffer, ExternalFramebufferPlugin, App, Assets, Default, Image, Option, Plugin (+5 more)

### Community 88 - "Macos App"
Cohesion: 0.27
Nodes (4): LoadKeyScript, Step, Bool, Int

### Community 89 - "Bus"
Cohesion: 0.20
Nodes (5): Default, Option, Self, TimexDock, TimexDockChunk

### Community 90 - "Z80"
Cohesion: 0.25
Nodes (10): contend_read_timing_adds_wait_without_mr(), FuseEventKind, interrupt_clears_q_before_scf(), interrupt_im2_uncontended_is_19_t(), interrupt_while_halted_does_not_skip_redirected_pc(), interrupt_while_halted_resumes_after_halt(), nmi_vectors_to_0066_and_preserves_iff2(), B (+2 more)

### Community 91 - "Z80"
Cohesion: 0.30
Nodes (14): adc8(), add8(), and8(), cp8(), dec8_flags(), inc8_flags(), or8(), parity() (+6 more)

### Community 92 - "Docs"
Cohesion: 0.16
Nodes (14): spec-chum-debugging skill, agent_server crate, control_plane crate, debug_cli crate, spec-chum-agent, spec-chum-debug, Agent Debug HTTP API, control_plane shared backend (+6 more)

### Community 93 - "Macos App"
Cohesion: 0.20
Nodes (5): LivingRoomDisplayView, Context, Int, DispatchWorkItem, NSView

### Community 94 - "Bus"
Cohesion: 0.22
Nodes (6): buttons_active_low(), delta_wraps_axes(), KempstonMouse, port_reads(), Option, Self

### Community 95 - "Formats"
Cohesion: 0.25
Nodes (11): apply_input_byte(), apply_matrix_and_kempston(), minimal_rzx(), parse_input_frames(), FnMut, Path, Result, Self (+3 more)

### Community 96 - "Host"
Cohesion: 0.15
Nodes (6): rom_search_roots(), BTreeMap, JoystickMode, PathBuf, UserMachineConfig, Vec

### Community 97 - "Agent"
Cohesion: 0.31
Nodes (11): EmbeddedServer, Arc, Option, Result, String, spawn(), spawn_fails_when_port_in_use(), spawn_from_env() (+3 more)

### Community 98 - "Bus"
Cohesion: 0.21
Nodes (5): divmmc_automap_via_notify_m1(), divmmc_conmem_overlays_via_bus48(), divmmc_control_beats_interface1_on_shared_e3(), divmmc_eeprom_fixture_automaps_when_present(), interface1_microdrive_ports_via_bus48()

### Community 99 - "Living"
Cohesion: 0.19
Nodes (5): perf_log_enabled(), rolling_window_resets(), RoomPerf, Instant, Option

### Community 100 - "Living"
Cohesion: 0.29
Nodes (10): chord_for(), chord_suppresses_caps(), letter_digit(), matrix_from_bevy(), push_unique(), quote_shift_is_sym_p(), ButtonInput, KeyCode (+2 more)

### Community 101 - "Living"
Cohesion: 0.22
Nodes (11): bloom_enabled(), env_truthy(), fxaa_enabled(), hybrid_enabled(), light_preset(), LightPreset, msaa_samples(), preset_label() (+3 more)

### Community 102 - "Misc"
Cohesion: 0.50
Nodes (13): agent_server, app, bus, control_plane, debug_cli, formats, host_api, living_room (+5 more)

### Community 103 - "Docs"
Cohesion: 0.18
Nodes (12): bus crate, formats crate, Hardware-faithful cycle-accurate accuracy, machine crate, tape crate, ula crate, z80 crate, Fuse Z80 test vectors (+4 more)

### Community 105 - "Host"
Cohesion: 0.26
Nodes (4): expected_rom_bytes(), pref_model_slug(), PrefModel, Model

### Community 106 - "Host"
Cohesion: 0.23
Nodes (3): Machine, Option, TypeLoadResult

### Community 107 - "Living"
Cohesion: 0.23
Nodes (12): animate_crt_params(), attach_crt_to_television(), CrtAttachedToTv, Entity, MeshMaterial3d, Option, Query, Res (+4 more)

### Community 108 - "Living"
Cohesion: 0.27
Nodes (11): CameraLocked, host_hotkeys(), Assets, ButtonInput, Image, KeyCode, Option, Res (+3 more)

### Community 109 - "Docs"
Cohesion: 0.22
Nodes (10): ./scripts/check.sh, host_api crate, living_room crate, C ABI FFI-only policy, Bevy 3D CRT living-room host, Dual-clock embed architecture, spec-chum-room, SpecChumMac SwiftUI shell (+2 more)

### Community 110 - "Slow Tests Sh"
Cohesion: 0.36
Nodes (10): ./scripts/run_slow_tests.sh, TDD expectations, Full slow test suite, CI z80doc job, ./scripts/run_system_tests.sh, minfo.tap, Third-party system tests, slow-tests feature (+2 more)

### Community 111 - "Living"
Cohesion: 0.27
Nodes (7): AssetPlugin, asset_plugin(), living_room_app(), resolve_asset_root(), App, PathBuf, main()

### Community 112 - "Control"
Cohesion: 0.24
Nodes (5): map_host_model_error(), From, Vec, WatchesResponse, WatchSpec

### Community 113 - "Host"
Cohesion: 0.27
Nodes (3): PrefAyStereo, AyStereoMode, JoystickMode

### Community 114 - "Bus"
Cohesion: 0.28
Nodes (3): bits_active_high(), Kempston, Self

### Community 115 - "Bus"
Cohesion: 0.31
Nodes (4): timex_2068_dock_ram_chunk_writable(), timex_2068_dock_spectrum_rom_pages_via_hsr(), timex_2068_exrom_pages_chunk0(), timex_2068_home_bank_spectrum_rom_replace()

### Community 116 - "Control"
Cohesion: 0.32
Nodes (4): encode_framebuffer_png(), parse_model_slug(), ApiResult, Vec

### Community 117 - "Living"
Cohesion: 0.29
Nodes (6): FileDialogPlugin, OpenMediaDialog, App, Plugin, ResMut, run_open_dialog()

### Community 118 - "Scripts"
Cohesion: 0.43
Nodes (6): append_bot_thread_from_comments(), apply_waiver_or_fail(), expect(), check_pr_reviews.sh script, classify_coderabbit_head_status(), pr_review_cr_classify.sh script

### Community 119 - "Docs"
Cohesion: 0.29
Nodes (7): ./scripts/fetch_roms.sh, Amstrad Lawson redistribution grant, ROM fetch policy, Timex TC2048 / TS2068, Cycle-accurate Z80, Spec Chum, Accurate ULA timing

### Community 120 - "App / Host"
Cohesion: 0.29
Nodes (6): main(), Result, default_prefs_path(), PrefsLock, PathBuf, Drop

### Community 121 - "App"
Cohesion: 0.43
Nodes (5): apply(), apply_does_not_panic_on_default_context(), clear_color(), panel_fill_is_opaque(), Context

### Community 122 - "Living"
Cohesion: 0.43
Nodes (6): main(), percentile_ms(), Duration, Vec, summarize(), varying_frame()

### Community 123 - "Living"
Cohesion: 0.38
Nodes (6): import_iosurface_texture(), c_void, RenderDevice, Result, String, Texture

### Community 124 - "Scripts"
Cohesion: 0.71
Nodes (6): checksum(), custom_loader_tap(), main(), make_code_tap(), CODE that `CALL 0556` (ROM LD-BYTES) for a following flag-0xC8 block. Models…, tap_block()

### Community 125 - "Pr Reviews Sh"
Cohesion: 0.33
Nodes (6): ./scripts/check_pr_reviews.sh, auto_review.enabled false, coderabbit-review label, CodeRabbit on-demand reviews, CodeRabbit merge gate, Bot review threads gate

### Community 126 - "App"
Cohesion: 0.47
Nodes (4): fit_size(), letterboxes_wide_window(), pillarboxes_tall_window(), Vec2

### Community 127 - "App"
Cohesion: 0.33
Nodes (4): HostAccess, Deref, DerefMut, MutexGuard

### Community 128 - "Living"
Cohesion: 0.40
Nodes (4): blit_to_crt(), dims_from_rgba_len(), Option, scale_hires_paper_to_crt()

### Community 129 - "Machine"
Cohesion: 0.40
Nodes (3): interface1_opcode_fetch_pages_shadow_rom(), interface1_rom_load_skips_cleanly_when_missing(), Interface1Error

### Community 130 - "Scripts"
Cohesion: 0.67
Nodes (5): checkout_sparse_repo(), copy_dir_roms(), copy_rom(), count_managed_roms(), fetch_roms.sh script

### Community 131 - "Docs"
Cohesion: 0.40
Nodes (5): app crate, Release process, egui/eframe primary host, Spec Chum.app release bundle, Release workflow

### Community 132 - "Docs"
Cohesion: 0.40
Nodes (5): Flash-load convenience exception, Flash-load at LD-BYTES 0x056C, type-load subcommand, attr_mark.tap, custom_loader.tap

### Community 134 - "Scripts"
Cohesion: 0.80
Nodes (4): fetch(), fetch_system_tests.sh script, sha256_of(), verify_sha()

### Community 135 - "Scripts"
Cohesion: 0.50
Nodes (4): run_one(), RUST_LOG, room_perf_matrix.sh script, SPEC_CHUM_ROOM_PERF_SOFT

### Community 138 - "Living"
Cohesion: 0.50
Nodes (3): CrtPlugin, App, Plugin

### Community 139 - "Living"
Cohesion: 0.50
Nodes (3): HybridPlugin, App, Plugin

### Community 140 - "Living"
Cohesion: 0.50
Nodes (3): PresentBlitPlugin, App, Plugin

### Community 141 - "Living"
Cohesion: 0.50
Nodes (3): RoomPlugin, App, Plugin

### Community 142 - "Living"
Cohesion: 0.50
Nodes (3): App, Plugin, UiOverlayPlugin

### Community 143 - "Scripts"
Cohesion: 0.67
Nodes (3): infer_crates(), RUSTFLAGS, check_crates.sh script

### Community 145 - "Skills Spec Chum"
Cohesion: 0.67
Nodes (3): SPEC_CHUM_AGENT_URL remote mode, Loopback HTTP 127.0.0.1:17384, sc_agent_embed_start

## Knowledge Gaps
- **146 isolated node(s):** `PackageDescription`, `Notification.Name`, `.isMenuTracking`, `.body`, `GameController` (+141 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 682 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **26 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `HostBridge` connect `macOS HostBridge` to `Macos App`, `Macos App`, `Macos App`, `Macos App`, `Macos App`, `Macos App`, `Macos App`, `Macos App`, `Macos App`, `Macos App`, `Macos App`, `Macos App`, `Macos App`, `Macos App`, `Macos App`?**
  _High betweenness centrality (0.224) - this node is a cross-community bridge._
- **Why does `HostSession` connect `Host Session` to `Host`, `Host API FFI`, `Host / Agent`, `Debug`, `Host`, `App`, `Control`, `Host`, `Host`, `Machine`, `Control`, `Control Plane`, `Control`, `Host`, `Living`, `App`?**
  _High betweenness centrality (0.213) - this node is a cross-community bridge._
- **Why does `Machine` connect `Machine Wiring` to `Z80`, `Machine`, `Bus`, `Bus`, `Bus`, `Ula`, `Machine`, `Machine`, `Bus`, `Machine`, `Machine / Tape`, `Machine`, `Machine`, `Machine`, `Bus`, `Bus`?**
  _High betweenness centrality (0.186) - this node is a cross-community bridge._
- **Are the 4 inferred relationships involving `HostBridge` (e.g. with `.livingRoomToolbar` and `TapeAudioPlayer`) actually correct?**
  _`HostBridge` has 4 INFERRED edges - model-reasoned connections that need verification._
- **What connects `PackageDescription`, `Notification.Name`, `.isMenuTracking` to the rest of the system?**
  _146 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Agent Debug API` be split into smaller, more focused modules?**
  _Cohesion score 0.055417700578990904 - nodes in this community are weakly interconnected._
- **Should `Host API FFI` be split into smaller, more focused modules?**
  _Cohesion score 0.060953041869072404 - nodes in this community are weakly interconnected._