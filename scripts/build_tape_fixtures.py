#!/usr/bin/env python3
"""Rebuild hand-crafted TAP fixtures under tests/fixtures/tape/ (freely redistributable)."""
from pathlib import Path

def checksum(data: bytes) -> int:
    c = 0
    for b in data:
        c ^= b
    return c

def tap_block(payload: bytes) -> bytes:
    return len(payload).to_bytes(2, "little") + payload

def make_code_tap(name: bytes, addr: int, code: bytes) -> bytes:
    assert len(name) == 10
    hdr = (
        bytes([0x00, 0x03])
        + name
        + len(code).to_bytes(2, "little")
        + addr.to_bytes(2, "little")
        + (0x8000).to_bytes(2, "little")
    )
    hdr += bytes([checksum(hdr)])
    data = bytes([0xFF]) + code
    data += bytes([checksum(data)])
    return tap_block(hdr) + tap_block(data)

def main() -> None:
    root = Path(__file__).resolve().parents[1] / "tests" / "fixtures" / "tape"
    root.mkdir(parents=True, exist_ok=True)
    # LD HL,4000h / LD (HL),42 / RET
    (root / "minimal_code.tap").write_bytes(
        make_code_tap(b"testcode  ", 0x8000, bytes([0x21, 0x00, 0x40, 0x36, 0x42, 0xC9]))
    )
    # LD HL,5800h / LD (HL),D7h / RET — first attribute cell
    (root / "attr_mark.tap").write_bytes(
        make_code_tap(b"attrmark  ", 0x8000, bytes([0x21, 0x00, 0x58, 0x36, 0xD7, 0xC9]))
    )
    body = bytes([0xF5, 0x22, 0x4F, 0x4B, 0x22, 0x0D])  # PRINT "OK"
    line = (10).to_bytes(2, "big") + len(body).to_bytes(2, "little") + body
    hdr = (
        bytes([0x00, 0x00])
        + b"printok   "
        + len(line).to_bytes(2, "little")
        + (0xFFFF).to_bytes(2, "little")
        + len(line).to_bytes(2, "little")
    )
    hdr += bytes([checksum(hdr)])
    data = bytes([0xFF]) + line
    data += bytes([checksum(data)])
    (root / "print_ok.tap").write_bytes(tap_block(hdr) + tap_block(data))
    print(f"wrote fixtures in {root}")

if __name__ == "__main__":
    main()
