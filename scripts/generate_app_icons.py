#!/usr/bin/env python3
"""Generate Spec Chum shared app icons (Refs #231).

Requires Pillow. Produces:
  packaging/icon/spec-chum-{256,512,1024}.png  — master sizes
  packaging/linux/spec-chum.png                — desktop / AppImage / .deb
  packaging/windows/spec-chum.ico              — PE + Inno Setup
  packaging/macos/AppIcon.icns                 — Spec Chum.app
  crates/app/assets/icon.png                   — egui window icon
  crates/app/assets/icon.ico                   — winres PE resource

Source: packaging/icon/spec-chum-1024.png, the Spectrum Enter icon master.
Platform exports and the macOS iconset are retained under packaging/icon so
regeneration does not depend on the original download.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]

MASTER = ROOT / "packaging/icon/spec-chum-1024.png"


def load_icon(size: int) -> Image.Image:
    """Resize the supplied Spectrum Enter master with high-quality filtering."""
    with Image.open(MASTER) as source:
        return source.convert("RGBA").resize((size, size), Image.Resampling.LANCZOS)


def write_png(path: Path, size: int, *, rgba: bool) -> None:
    im = load_icon(size)
    path.parent.mkdir(parents=True, exist_ok=True)
    if rgba:
        im.save(path, optimize=True)
    else:
        im.convert("RGB").save(path, optimize=True)
    print(f"wrote {path.relative_to(ROOT)} ({size}x{size})")


def write_ico(path: Path) -> None:
    source = ROOT / "packaging/icon/SpectrumEnter.ico"
    path.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, path)
    print(f"copied {path.relative_to(ROOT)} from {source.relative_to(ROOT)}")


def write_icns(path: Path) -> None:
    source = ROOT / "packaging/icon/SpectrumEnter.icns"
    if sys.platform != "darwin":
        shutil.copyfile(source, path)
        print(f"copied {path.relative_to(ROOT)} from {source.relative_to(ROOT)}")
        return

    mapping = [
        ("icon_16x16.png", 16),
        ("icon_16x16@2x.png", 32),
        ("icon_32x32.png", 32),
        ("icon_32x32@2x.png", 64),
        ("icon_128x128.png", 128),
        ("icon_128x128@2x.png", 256),
        ("icon_256x256.png", 256),
        ("icon_256x256@2x.png", 512),
        ("icon_512x512.png", 512),
        ("icon_512x512@2x.png", 1024),
    ]
    with tempfile.TemporaryDirectory(prefix="spec-chum-iconset-") as tmp:
        iconset = Path(tmp) / "AppIcon.iconset"
        iconset.mkdir()
        for name, sz in mapping:
            shutil.copyfile(ROOT / "packaging/icon/iconset" / name, iconset / name)
        path.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(path)],
            check=True,
        )
    print(f"wrote {path.relative_to(ROOT)}")


def main() -> int:
    if not MASTER.is_file():
        raise SystemExit(f"missing icon master: {MASTER}")
    print(f"using {MASTER.relative_to(ROOT)}")
    write_png(ROOT / "packaging/icon/spec-chum-256.png", 256, rgba=True)
    write_png(ROOT / "packaging/icon/spec-chum-512.png", 512, rgba=True)
    # RGBA for egui IconData (png crate decodes without expansion).
    write_png(ROOT / "crates/app/assets/icon.png", 256, rgba=True)
    # Keep the Linux release export supplied with the pack's platform assets.
    shutil.copyfile(
        ROOT / "packaging/icon/linux/spec-chum.png",
        ROOT / "packaging/linux/spec-chum.png",
    )
    write_ico(ROOT / "packaging/windows/spec-chum.ico")
    shutil.copyfile(
        ROOT / "packaging/windows/spec-chum.ico",
        ROOT / "crates/app/assets/icon.ico",
    )
    print("wrote crates/app/assets/icon.ico")
    write_icns(ROOT / "packaging/macos/AppIcon.icns")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
