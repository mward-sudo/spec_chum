# Tape fixtures

Freely redistributable TAP/TZX images for automated tests.

Rebuild **TAP** fixtures only with:

```bash
python3 scripts/build_tape_fixtures.py
```

That script writes `minimal_code.tap`, `attr_mark.tap`, `print_ok.tap`, and
`custom_loader.tap`. It does not rebuild `minimal.tzx` (hand-maintained for TZX unit tests).

| File | Purpose |
| --- | --- |
| `minimal_code.tap` | CODE header + 6-byte routine at `0x8000`: `LD HL,4000 / LD (HL),42 / RET` |
| `attr_mark.tap` | CODE at `0x8000`: writes `0xD7` to attribute `0x5800` then `RET` (visible load mark). Load with `LOAD "" CODE`. |
| `print_ok.tap` | 1-line BASIC `10 PRINT "OK"` (header name `printok`). Load with `LOAD ""`. |
| `custom_loader.tap` | CODE @`8000` calls ROM `LD-BYTES` for a following `0xC8` flag block and loads byte `0xA5` at `0x9000` (Boggit-style custom flag). Load with `LOAD "" CODE`, then `RANDOMIZE USR 32768`. |
| `minimal.tzx` | Minimal standard-speed TZX wrapper used by TZX unit tests (not rebuilt by the script above) |

Do not add commercial game TAPs.

## TZX Loop Start/End regression (#372)

CI always covers synthetic Loop Start (`0x24`) / Loop End (`0x25`) via:

- `cargo test -p tape loop_start_end_expands_pure_tone` — pulse schedule expands N×
- `cargo test -p host_api open_tape_expands_tzx_loop_blocks` — SpecChumMac `open_tape` path

Prior fixtures (`minimal.tzx`, Boggit-style standard-only) never exercised loop blocks, so
commercial Speedlock TZXs like Arkanoid failed open with no CI signal.

## TZX block-ID capability matrix (#374)

Beyond accuracy tiers ([docs/TESTING.md](../../../docs/TESTING.md)), CI runs a **synthetic
block-ID matrix** so commercial/pulse TZX paths are not manual-only:

| Gate | Command |
| --- | --- |
| Supported IDs parse | `cargo test -p tape tzx_block_matrix_supported_ids_parse` |
| Unsupported IDs Err | `cargo test -p tape tzx_block_matrix_unsupported_ids_fail_loudly` |
| TAP scan no truncate | `cargo test -p tape to_tap_image_errors_on_unsupported` / `to_tap_image_keeps_id10_across_skip_markers` |
| Host open / `has_tape` | `cargo test -p host_api open_tape_tzx_block_matrix_supported_families` |
| Unsupported leaves deck | `cargo test -p host_api open_tape_tzx_block_matrix_unsupported_leaves_deck` |

Supported today: `0x10`–`0x14`, `0x20`, Group/Text/Archive/Hardware/Custom/Glue skip markers,
Loop `0x24`/`0x25`. Intentionally unsupported (hard error with ID in message): Direct `0x15`,
CSW `0x18`, GDB `0x19`, Jump/Call/Select/`Stop if 48K`/`Set signal level` — listed as known gaps
under [#374](https://github.com/mward-sudo/spec_chum/issues/374).

## Content identity (#366)

Fixture digests are registered in the offline catalogue
(`formats::media_identity`) so hosts can show human titles (e.g. `PRINT "OK"`)
instead of filenames. See [`docs/TAPE_IDENTITY.md`](../../../docs/TAPE_IDENTITY.md).

## Load matrix (CI)

`cargo test -p machine --lib matrix` covers:

| Fixture | Models | Instant | EAR speeds |
| --- | --- | --- | --- |
| `attr_mark.tap` | 48K, 128K, +3 | yes | 1/2/5/10/20 (48K); 2/5/10/20 (128K/+3) |
| `custom_loader.tap` | 48K, 128K, +3 | yes | 1 (48K) + 2/5/10/20 (all) |

Set `SPEC_CHUM_FULL_TAPE_MATRIX=1` to also run EAR@1 on 128K/+3 (slow).

## Local repro (not in git)

Optional commercial tape for optional local tests (never commit):

```bash
export SPEC_CHUM_BOGGIT_TZX="$HOME/Downloads/BoggitThe/The Boggit - Side 1.tzx"
# Instant + EAR@2/5/10/20 on 48K/128K/+3; add FULL for EAR@1
SPEC_CHUM_FULL_TAPE_MATRIX=1 cargo test -p machine --lib boggit -- --nocapture
```

Speedlock / Loop Start (`0x24`) example (optional local only):

```bash
# ~/Downloads/Arkanoid.tzx — HostSession open + TzxPlayer parse
cargo test -p tape arkanoid_downloads -- --nocapture
cargo test -p tape arkanoid_pause_polarity -- --nocapture
cargo test -p host_api open_local_arkanoid -- --nocapture
# EAR Play past the Speedlock sampler (#379 / #380)
cargo test -p host_api --release --lib arkanoid_ear_leaves_sampler -- --ignored --nocapture
# Protection completes at 1x after the deck finishes (#390)
cargo test -p host_api --release --lib arkanoid_speedlock_completes_at_realtime -- --ignored --nocapture
```

TZX pause polarity follows Fuse `force_low` / `LEVEL_LOW` (absolute low on the
first edge after a non-zero pause). A bad `LEVEL_LOW` that emitted an *extra*
low at tape start inverted the whole EAR schedule and stuck Arkanoid at `$FD2A`.

### The post-tape delay does not need turbo (#390)

EAR `speed` is a **loading** convenience: it stops applying the moment the deck
finishes, including for programs that run with interrupts disabled from upper
RAM (Arkanoid does). `Machine::effective_speed_multiplier()` reports the rate
actually being run, and both hosts show that instead of the setting.

Measured with `arkanoid_realtime_gate` (steps instructions, times landmarks in
T-states):

| Run | Result |
| --- | --- |
| 1x, **no** key input | `EI` at **20.05 s** of Spectrum time, 124 delay stages |
| 1x, Space tapped 120 ms every 700 ms | gate passed 0.8 s, `$83DF` main loop at **20.7 s** |
| 64x post-tape turbo (old gate), 4000 s Spectrum | never leaves protection; PC drifts in `$F1xx` |

So the protection is a ~20 s multi-stage delay that ends on its own — the
"~4.5 hours of Spectrum time" figure recorded earlier was an artifact of the
turbo path itself: multi-frame bursts plus a synthetic Space auto-ack perturbed
the loop so it never completed, which then looked like it needed *more* turbo.
The auto-ack, `SPEC_CHUM_SPEEDLOCK_SNAP`, `SPEC_CHUM_SPEEDLOCK_BOOST` and the
`speedlock_stage()` chrome are gone with it.

After EAR exhaust, Arkanoid sits in a multi-stage Speedlock DI delay at `$F448`
(`LD E,IXH` at `$F44E`, not a CALL). Outer stub after `$F408`: `CALL $8224`
polls `IN A,(C)` with `BC=$00FE` — no key aborts with `E=$FF` and restarts the
delay, any key continues into decrypt / game setup (`$8230` / `$93xx`).

### The load path is not the defect (#379)

`arkanoid_ram_diff` loads `Arkanoid.tzx` through Spec Chum’s EAR path and diffs
the resulting `$4000-$FFFF` image against a Fuse snapshot of the **same** tape:

- **42 differing bytes out of 49152**, in 22 short runs.
- Every run is loader scratch (`$94C8-$94F8`, `$956D`, `$9F03`, `$F13A-$F148`,
  `$F3D9-$F3E0`, `$FDDE`, `$FFBE`) or BASIC system vars (`$5C02-$5C78`) — i.e.
  state that legitimately differs between two sample points of the same loop.
- No differing run anywhere in the loaded code or screen bitmap.

So the TZX/EAR bitstream is decoded correctly and **the remaining #379 failure is
post-load protection execution**, not tape decoding. Retire load-path hypotheses.

### Decoded Speedlock 2 protection (#379)

`arkanoid_stub_trace` breaks on the decision points (frame sampling only ever
caught the delay nest) and disassembles the live, already-decrypted bytes:

```text
8138  LD ($94C8),SP        ; outer driver
813C  LD A,$04
813E  LD ($9F03),A         ; run the stub 4×
8141  CALL $F3C1
8144  DI
8145  LD A,($9F03) / DEC A / LD ($9F03),A
814C  JR NZ,$8141
814E  LD ($956F),A         ; A = 0 → clears the “signature seen” flag

F3CD  DI
F3CE  CALL $F408           ; long border-delay nest ($F448 … RET $F476)
F3D1  CALL $8224           ; any-key gate
F3D4  INC E
F3D5  JR Z,$F3CE           ; E=$FF ⇒ spin forever

8211  IN A,(C) / OR $E0 / INC A       ; wait for key *release*, then
821B  LD ($F3D2),BC        ; repoint the CALL at $F3D1 to $8224
821F  POP AF / POP BC / LD E,$FF / RET

8224  LD BC,$00FE / IN A,(C) / OR $E0 / INC A
822E  JR Z,$821F           ; no key ⇒ E=$FF ⇒ spin
8230  LD HL,($94F6) / INC HL
823A  LD DE,$9570 / LD B,$06
823F  LD A,(DE) / CP (HL) / JR NZ,$824A / INC HL / INC DE / DJNZ
8247  LD ($956F),A         ; only when all six bytes match
824A  LD SP,($94C8)        ; stack switch — never returns to $F3D4
824E  LD A,($956F) / OR A / JR Z,$8280   ; miss ⇒ wipe screen, retry
```

The six expected bytes at `$9570` are ASCII **`"PBRAIN"`**. The gate therefore
only clears when `"PBRAIN"` sits at `($94F6)+1`. Measured behaviour:

- `($94F6)` is a **screen cursor**, ping-ponging around `$4E00-$58FF` (deltas of
  ±200 and ±2500 per 25 host frames). It is not a search that converges.
- `"PBRAIN"` occurs exactly once in RAM — in the `$9570` expected-value table
  itself — in **both** the Spec Chum and Fuse images.
- `$956F` is 0 in both images, so both take the `$8280` miss path.

The miss path at `$8280` wipes the screen (`$8368` fills `$4000-$5AFF`, attrs
`$47`) and re-enters the delay, which is what made turbo soaks look like an
endless `delay-f448` ↔ `decrypt-93` oscillation. It is not the boot gate: the
`"PBRAIN"` compare is a cheat / name check, and at 1× the protection completes
and starts the game without it ever matching (#390).

Local helpers (not CI — copyrighted media / large artifacts stay under `tmp/`):

```bash
# Time the post-tape protection at a true 1x (needs ~/Downloads/Arkanoid.tzx)
cargo run -p host_api --release --example arkanoid_realtime_gate -- ~/Downloads/Arkanoid.tzx

# Decision-point breakpoints + live disassembly of the protection
cargo run -p host_api --release --example arkanoid_stub_trace -- ~/Downloads/Arkanoid.tzx 40
# …or `0` for cursor mode (watch ($94F6) vs the $9570 table)

# Prove the load path: Spec Chum EAR image vs a Fuse snapshot of the same tape
cargo run -p host_api --release --example arkanoid_ram_diff -- \
  tmp/fuse_oracle/arkanoid_fuse_postload.z80 ~/Downloads/Arkanoid.tzx
```

`.z80` load now restores **R bit 7** from header byte 12 bit 0 (FAQ) so
snapshot `LD A,R` keys match Fuse when R7 is set.
### How to load The Boggit on 128K at 1×

1. Model **128K**, insert Side 1 TZX (converted to TAP automatically).
2. Turn **Instant flash-load** **off** for a pure EAR experience, **or** leave it **on** (ROM/48 BASIC blocks flash; custom `0xC8` blocks still need the RAM loader + EAR/Instant trap).
3. Prefer **Tape Loader** from the 128 menu (Enter), **or** 48 BASIC then `LOAD ""` (**not** `LOAD "" CODE` — Side 1 starts with a **PROGRAM**).
4. Press **Play** when the border goes red/cyan / the loader waits.
5. EAR speed **1×** is realtime; higher speeds run N Spectrum frames per host tick while Play is active (wall-clock ≈ realtime / N; pulse widths stay ROM-accurate).

Debug / observability: see [`docs/DEBUGGING.md`](../../../docs/DEBUGGING.md).

