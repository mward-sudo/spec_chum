# Spectrum Next timing comparisons

This document defines a repeatable, opt-in comparison workflow for timing-sensitive Spectrum Next behavior. It does not claim parity with MAME or ZEsarUX until a case has been run against an identified build of each reference.

## Local deterministic baseline

From a clean checkout with Rust installed, run:

```sh
CARGO_TARGET_DIR=/tmp/spec-chum-next-timing-target cargo test -p machine next:: --release
```

This runs the Next-machine unit suite without ROM images, emulator binaries, or network access. To keep generated build files out of the checkout, set `CARGO_TARGET_DIR` to a writable scratch directory as shown. The test names are stable selectors for the initial comparison cases:

| Case | Spec Chum oracle | Observable result |
| --- | --- | --- |
| Display timing and frame interrupt | `frame_interrupt_uses_selected_display_timing` | Frame lengths and interrupt acceptance boundary for timing IDs 0–7 |
| CPU turbo and CTC clock | `cpu_turbo_runs_more_instructions_per_frame_without_speeding_up_ctc` | CPU T-states at all four programmed speeds, stable CTC decrement per 9,600 master ticks, and frame period |
| CTC interrupt service | `ctc_hardware_im2_interrupt_runs_guest_handler_and_reti_releases_service` | Guest-programmed timer-to-counter cascade, channel-specific IM2 vector targets, handler markers, RETI service release, and the next channel interrupt |
| Timed Copper video change | `copper_wait_move_updates_layer2_in_the_current_frame` | Framebuffer pixels before and after a raster-timed MOVE |
| Timed DAC/audio output | `next_machine_samples_held_dac_values_at_their_tstate_and_gates_output` | DAC samples and output gate at the written machine time |

These cases are executable local baselines only. Their assertions encode Spec Chum's intended model, not reference-emulator observations.

## Differential procedure

1. Record the exact Spec Chum commit, MAME release/commit, ZEsarUX release/commit, host OS, machine model, Next core version, ROM image provenance and SHA256, and invocation/configuration for all three programs.
2. Use a deterministic guest program for each selected case. Keep the program source, assembled bytes, load address, initial registers/memory, and expected stop condition together in a fixture directory. Do not use a proprietary ROM or binary emulator dependency.
3. Capture the same guest-visible observations from each emulator: instruction/master-clock boundary, selected NextRegs/CTC state, memory marker, and (where relevant) raw framebuffer or timestamped audio/DMA event data. A screenshot comparison alone is labeled manual and cannot establish cycle-level agreement.
4. Save unmodified raw output and a normalized machine-readable result (JSON or CSV) with each run. The normalizer must not round away timing differences. Record tool commands and hashes beside the result.
5. Compare outputs with an explicit exact/tolerance rule for each field. Report `match`, `mismatch`, or `unavailable`; explain missing capabilities rather than silently omitting a reference.
6. Classify each mismatch as a Spec Chum defect, known reference/specification difference, or deferred limitation. Only open a follow-up issue when the discrepancy is actionable and has a deterministic acceptance check.

## Reference availability and limits

MAME and ZEsarUX are open-source projects, but neither reference binary is bundled or fetched by this workflow. Their source/build revisions and supported Spectrum Next capabilities vary. This procedure therefore does not require installation, network access, or proprietary assets; a maintainer who has a suitable local build can record an explicit run. Until both references and all selected cases have captured outputs, the differential result is **not run**, not passed.

The initial local baseline currently covers timing, CTC interrupt service, raster-timed video, and DAC sampling. It does not yet provide a portable capture adapter for MAME/ZEsarUX or checked-in reference traces. Reference comparison and any fixture tooling remain follow-up work until each target's automation interface and output fidelity are verified.

## Reproducibility record template

```text
Case:
Spec Chum commit:
MAME version/commit or unavailable:
ZEsarUX version/commit or unavailable:
Host OS and architecture:
Machine model / Next core version:
ROM source and SHA256:
Guest fixture source and SHA256:
Commands and configuration:
Observed outputs and artifact paths:
Comparison rule and result:
Classification / follow-up issue:
```
