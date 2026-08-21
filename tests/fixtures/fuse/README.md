# Fuse Z80 test vectors

`tests.in` and `tests.expected` are from the [Fuse](https://fuse-emulator.sourceforge.net/) ZX Spectrum emulator (`z80/tests/`), GPL-licensed.

They are used only as test fixtures for Spec Chum's Z80 core and are not linked into the emulator binary.

A few `*_1` / repeat-cycle expected lines (`INIR`/`INDR`/`OTIR`/`OTDR`/`CPDR` at 21 T) were updated to match post-2018 undocumented block-repeat flag / MEMPTR behaviour (Banks / Ped7g / z80test `z80full`), which Fuse’s original vectors predate.
