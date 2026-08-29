# System ROMs — fetch, copyright, attribution

Spec Chum’s MIT `LICENSE` covers **our code only**. Spectrum (and related)
firmware images are separate works.

## Official Sinclair / Amstrad Spectrum ROMs

Cliff Lawson (Amstrad plc, 1999-08-31) stated that Amstrad are happy for
emulator writers to include images of their copyrighted Spectrum ROM code
**as long as the (c)opyright messages inside the images are not altered**, and
asked that the program/manual note:

> Amstrad have kindly given their permission for the redistribution of their
> copyrighted material but retain that copyright.

That grant covers the classic Spectrum family ROMs Spec Chum fetches today
(16K / 48K / 128K / grey +2 / +2A / +3). Amstrad (and Sinclair as original
author) retain copyright. Do **not** strip or patch the copyright strings
inside `.rom` files.

### What this project does today

- System ROMs are **not** committed to git and are **not** attached to GitHub
  Releases ([RELEASE.md](RELEASE.md)).
- Fetch official images with `./scripts/fetch_roms.sh` (pinned sparse checkout
  of [spectrumforeveryone/zx-roms](https://github.com/spectrumforeveryone/zx-roms)).

If we ever ship ROM bytes with a build, the Lawson notice above remains
required, and in-image copyright messages must stay intact.

## Not covered by the Amstrad grant

Supply your own images (or use a separate clear grant documented when we add
fetch support). Examples:

| Firmware | Notes |
| --- | --- |
| Interface 1 / Microdrive | Not Amstrad’s Spectrum grant |
| Multiface 1 / 128 | User-provided |
| TR-DOS / Beta Disk | User-provided |
| ESXDOS / DivMMC EEPROM | Confirm rights before shipping |
| Pentagon / Scorpion | Not licensed by Amstrad |
| Regional clones (Didaktik, TK90X, …) | Rights vary; usually user-provided |
| ZX80 / ZX81 | Outside the Spectrum grant |

Peripheral attach UX: Multiface ([MULTIFACE.md](MULTIFACE.md)), Interface 1,
Beta, DivMMC — see GitHub issues #137–#140 / #169.

## Other ROMs with their own grants

When Spec Chum adds fetch/ship for non-Amstrad images that **do** have a clear
redistribution grant (e.g. Timex, +3e, OpenSE, Datel +D/DISCiPLE, SpeccyBoot),
document that grant next to the fetch path — do **not** treat the Lawson
sentence alone as covering them. Tracking: [#190](https://github.com/mward-sudo/spec_chum/issues/190).

Spectrum Next firmware uses **The Next Licence** (separate product) — see
[#191](https://github.com/mward-sudo/spec_chum/issues/191).

## References

- [DeZog `amstrad-rom-permissions.txt`](https://github.com/maziac/DeZog/blob/main/documentation/amstrad-rom-permissions.txt) (full Lawson post)
- Spectaculator / Debian `spectrum-roms` legal notes (same wording)
