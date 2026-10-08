#!/usr/bin/env python3
"""Compare one tiny Z80N-compatible cycle probe in MAME and ZEsarUX."""

from __future__ import annotations

import argparse
import hashlib
import os
import platform
import re
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path


MARKER = 0x2A
EXPECTED_BASE_TSTATES = 20
EXPECTED_PROBE_TSTATES = 180
EXPECTED_MAME_CTC_STATUS = 0x01
EXPECTED_ZESARUX_CTC_STATUS = 0x00
BASELINE_STOP = 0xC005
PROBE_STOP = 0xC02A
PROBE_STEP_COUNT = 16
ZRCP_PORT = 10000
MAME_BOOTROM_SHA1 = "8b3c2a301f486904d1c74929b94845a7731bf230"


def make_nex() -> bytes:
    header = bytearray(512)
    header[:4] = b"Next"
    header[4:8] = b"V1.2"
    header[9] = 1  # One 16 KiB memory bank follows.
    header[10] = 0x80  # No optional loading-screen blocks.
    header[12:14] = (0xFFFF).to_bytes(2, "little")
    header[14:16] = (0xC000).to_bytes(2, "little")
    header[20] = 1  # Bank 2 is present.
    header[134] = 1  # Preserve the NextRegs selected by the machine setup.
    header[135:138] = bytes((3, 1, 0))  # Require Next core 3.01.00.
    header[139] = 2  # Map bank 2 into the entry slot.

    bank = bytearray(16_384)
    # Save a marker, enable CTC0, run it once, read status, then loop.
    program = bytes(
        (
            0x3E, MARKER, 0x32, 0x00, 0xC1,  # LD A,$2A; LD ($C100),A
            0x01, 0x3B, 0x24, 0x3E, 0xC5, 0xED, 0x79,  # Select CTC interrupt enable
            0x3E, 0x01, 0xED, 0x79,  # Enable channel 0
            0x01, 0x3B, 0x18, 0x3E, 0x85, 0xED, 0x79,  # OUT CTC0 control
            0x3E, 0x01, 0xED, 0x79,  # OUT CTC0 time constant 1
            0x01, 0x3B, 0x24, 0x3E, 0xC9, 0xED, 0x79,  # Select NEXTREG $C9
            0x01, 0x3B, 0x25, 0xED, 0x78,  # Read NEXTREG $C9 via $253B
            0x32, 0x01, 0xC1,  # LD ($C101),A
            0xC3, 0x2A, 0xC0,  # JP $C02A
        )
    )
    bank[: len(program)] = program
    return bytes(header + bank)


def extract_mame_bootrom(source: Path, rompath: Path) -> None:
    vhdl = source.read_text(encoding="utf-8")
    rom_block = re.search(r"constant ROM\s*:\s*ROM_ARRAY\s*:=\s*\((.*?)\);", vhdl, re.DOTALL)
    if not rom_block:
        raise RuntimeError(f"No ROM_ARRAY initializer found in {source}")
    image = bytes(int(value, 16) for value in re.findall(r'x"([0-9A-Fa-f]{2})"', rom_block.group(1)))
    if len(image) != 8192 or hashlib.sha1(image).hexdigest() != MAME_BOOTROM_SHA1:
        raise RuntimeError("bootrom.vhd does not contain the MAME v30100 8 KiB image")
    target = rompath / "tbblue" / "boot-30100.bin"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(image)


def mame_lua() -> str:
    return r'''local debugger = manager.machine.debugger
debugger:command('bpset 0xc000,,{bpdisable 1 ; printf "PROBE_START %d\\n",totalcycles ; g}')
debugger:command('bpset 0xc005,,{bpdisable 2 ; printf "PROBE_BASE %d\\n",totalcycles ; g}')
debugger:command('bpset 0xc02a,,{bpdisable 3 ; printf "PROBE_STOP %d %d %d\\n",totalcycles,b@0xc100,b@0xc101 ; g}')
debugger.execution_state = "run"
local function poll()
  local start, base, stop, marker, ctc_status
  for i = 1, #debugger.consolelog do
    local line = debugger.consolelog[i]
    start = start or string.match(line, "PROBE_START (%d+)")
    base = base or string.match(line, "PROBE_BASE (%d+)")
    local cycles, value, status = string.match(line, "PROBE_STOP (%d+) (%d+) (%d+)")
    stop = stop or cycles
    marker = marker or value
    ctc_status = ctc_status or status
  end
  if start and base and stop and marker and ctc_status then
    print("MAME_RESULT base=" .. (tonumber(base) - tonumber(start)) ..
      " tstates=" .. (tonumber(stop) - tonumber(start)) ..
      " marker=" .. marker .. " ctc=" .. ctc_status)
    manager.machine:exit()
  end
end
emu.add_machine_frame_notifier(poll)
'''


def mame_result(executable: str, rompath: Path, work: Path, nex: Path) -> tuple[int, int, int, int]:
    script = work / "capture.lua"
    script.write_text(mame_lua(), encoding="utf-8")
    isolated = work / "mame"
    options = [
        "-noreadconfig",
        "-video", "none",
        "-sound", "none",
        "-nothrottle",
        "-debug",
        "-debugger", "none",
        "-autoboot_script", str(script),
        "-seconds_to_run", "1",
        "-rompath", str(rompath),
        "-bios", "v30100",
        "-cfg_directory", str(isolated / "cfg"),
        "-nvram_directory", str(isolated / "nvram"),
        "-state_directory", str(isolated / "state"),
        "-snapshot_directory", str(isolated / "snapshots"),
        "tbblue",
        "-snapshot", str(nex),
    ]
    env = os.environ.copy()
    env["SDL_VIDEODRIVER"] = "dummy"
    result = subprocess.run(
        [executable, *options], capture_output=True, text=True, timeout=20, env=env
    )
    output = result.stdout + result.stderr
    match = re.search(
        r"MAME_RESULT base=(\d+) tstates=(\d+) marker=(\d+) ctc=(\d+)", output
    )
    if result.returncode != 0 or not match:
        raise RuntimeError(f"MAME probe failed (exit {result.returncode}):\n{output}")
    base_tstates, tstates, marker, ctc_status = map(int, match.groups())
    if marker != MARKER:
        raise RuntimeError(f"MAME returned inconsistent probe data:\n{output}")
    return base_tstates, tstates, marker, ctc_status


def zesarux_command(sock: socket.socket, command: str) -> str:
    sock.sendall(command.encode("utf-8") + b"\n")
    chunks = []
    last_data = time.monotonic()
    deadline = last_data + 5
    while time.monotonic() < deadline:
        try:
            data = sock.recv(4096)
        except socket.timeout:
            if chunks and time.monotonic() - last_data >= 0.2:
                break
            continue
        if not data:
            break
        chunks.append(data)
        last_data = time.monotonic()
    return b"".join(chunks).decode("utf-8", errors="replace")


def zesarux_result(app: str, nex: Path) -> tuple[int, int, int, int]:
    with socket.socket() as probe:
        probe.settimeout(0.2)
        if probe.connect_ex(("127.0.0.1", ZRCP_PORT)) == 0:
            raise RuntimeError(
                f"ZRCP port {ZRCP_PORT} is already in use; stop the existing instance first"
            )

    args = ["--noconfigfile", "--machine", "tbblue", "--vo", "null", "--ao", "null", "--enable-remoteprotocol"]
    if platform.system() == "Darwin":
        if not app.lower().endswith(".app"):
            raise RuntimeError("On macOS, pass the ZEsarUX .app bundle; do not launch its executable directly")
        launch = ["open", "-n", "-g", "-j", "-W", "-a", app, "--args", *args]
    else:
        launch = [app, *args]
    process = subprocess.Popen(launch, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    sock = None
    try:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            try:
                sock = socket.create_connection(("127.0.0.1", ZRCP_PORT), timeout=0.5)
                break
            except OSError:
                if process.poll() is not None:
                    raise RuntimeError(f"ZEsarUX exited with status {process.returncode}")
                time.sleep(0.2)
        if sock is None:
            raise RuntimeError("ZEsarUX did not open its local ZRCP port within 15 seconds")

        sock.settimeout(0.1)
        zesarux_command(sock, "")
        zesarux_command(sock, "enter-cpu-step")
        zesarux_command(sock, f"smartload {nex}")
        registers = zesarux_command(sock, "get-registers")
        if not re.search(r"PC=c000\b", registers, re.IGNORECASE):
            raise RuntimeError(f"ZEsarUX did not stop at the fixture entry point:\n{registers}")
        zesarux_command(sock, "reset-tstates-partial")
        baseline_steps = [
            zesarux_command(sock, "cpu-step"),
            zesarux_command(sock, "cpu-step"),
        ]
        baseline_registers = zesarux_command(sock, "get-registers")
        if not re.search(r"PC=c005\b", baseline_registers, re.IGNORECASE):
            raise RuntimeError(f"ZEsarUX did not reach the baseline boundary:\n{baseline_registers}")
        baseline_tstates = re.findall(r"TSTATES:\s*(\d+)", "".join(baseline_steps))
        probe_steps = [
            zesarux_command(sock, "cpu-step") for _ in range(PROBE_STEP_COUNT)
        ]
        registers = zesarux_command(sock, "get-registers")
        marker_memory = zesarux_command(sock, "read-memory C100H 1")
        status_memory = zesarux_command(sock, "read-memory C101H 1")
        probe_tstates = re.findall(r"TSTATES:\s*(\d+)", "".join(probe_steps))
        marker = re.search(r"\b([0-9A-F]{2})\b", marker_memory, re.IGNORECASE)
        ctc_status = re.search(r"\b([0-9A-F]{2})\b", status_memory, re.IGNORECASE)
        if not baseline_tstates or not probe_tstates or not marker or not ctc_status:
            raise RuntimeError(
                f"ZEsarUX did not return timing and marker data:\n{baseline_steps}{probe_steps}{marker_memory}{status_memory}"
            )
        if not re.search(rf"PC={PROBE_STOP:04x}\b", registers, re.IGNORECASE):
            raise RuntimeError(f"ZEsarUX did not reach the probe boundary:\n{registers}")
        return (
            int(baseline_tstates[-1]),
            int(probe_tstates[-1]),
            int(marker.group(1), 16),
            int(ctc_status.group(1), 16),
        )
    finally:
        if sock is not None:
            try:
                zesarux_command(sock, "exit-emulator")
            except OSError:
                pass
            finally:
                sock.close()
        if process.poll() is None:
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mame", default="mame", help="MAME executable")
    parser.add_argument("--bootrom-vhdl", type=Path, required=True, help="official core source containing the MAME v30100 ROM")
    parser.add_argument("--zesarux", required=True, help="ZEsarUX executable (non-macOS) or macOS .app bundle")
    args = parser.parse_args()

    with tempfile.TemporaryDirectory(prefix="next-probe-") as temp:
        work = Path(temp)
        nex = work / "cycle-probe.nex"
        nex.write_bytes(make_nex())
        rompath = work / "mame-roms"
        extract_mame_bootrom(args.bootrom_vhdl, rompath)
        mame_base, mame_cycles, mame_marker, mame_ctc = mame_result(
            args.mame, rompath, work, nex
        )
        zesarux_base, zesarux_cycles, zesarux_marker, zesarux_ctc = zesarux_result(
            args.zesarux, nex
        )
    print(
        f"MAME: base={mame_base} probe={mame_cycles} marker=0x{mame_marker:02X} "
        f"ctc_status=0x{mame_ctc:02X}"
    )
    print(
        f"ZEsarUX: base={zesarux_base} probe={zesarux_cycles} "
        f"marker=0x{zesarux_marker:02X} ctc_status=0x{zesarux_ctc:02X}"
    )
    expected_timing_and_marker = (EXPECTED_BASE_TSTATES, EXPECTED_PROBE_TSTATES, MARKER)
    if (mame_base, mame_cycles, mame_marker) != expected_timing_and_marker:
        raise RuntimeError("MAME result differs from the probe's expected output")
    if (zesarux_base, zesarux_cycles, zesarux_marker) != expected_timing_and_marker:
        raise RuntimeError("ZEsarUX result differs from the probe's expected output")
    if mame_ctc != EXPECTED_MAME_CTC_STATUS:
        raise RuntimeError("MAME CTC status differs from the recorded observation")
    if zesarux_ctc != EXPECTED_ZESARUX_CTC_STATUS:
        raise RuntimeError("ZEsarUX CTC status differs from the recorded observation")
    print(
        "OBSERVED: both timing paths report 20/180 T-states and marker 0x2A; "
        "CTC status differs (MAME=0x01, ZEsarUX=0x00)"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1) from error
