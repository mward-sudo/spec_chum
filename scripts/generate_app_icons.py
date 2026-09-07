#!/usr/bin/env python3
"""Generate Spec Chum shared app icons (Refs #231).

Requires Pillow. Produces:
  packaging/icon/spec-chum-{256,512,1024}.png  — master sizes
  packaging/linux/spec-chum.png                — desktop / AppImage / .deb
  packaging/windows/spec-chum.ico              — PE + Inno Setup
  packaging/macos/AppIcon.icns                 — Spec Chum.app (via iconutil)
  crates/app/assets/icon.png                   — egui window icon
  crates/app/assets/icon.ico                   — winres PE resource

Design: dark CRT bezel, green BASIC block cursor, classic Spectrum rainbow stripe.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]

BLACK = (0, 0, 0, 255)
BEZEL = (48, 48, 48, 255)
SCREEN = (8, 24, 20, 255)
CURSOR = (0, 216, 0, 255)
WHITE = (220, 220, 220, 255)
RAINBOW = [
    (216, 0, 0, 255),  # red
    (252, 216, 0, 255),  # yellow
    (0, 216, 0, 255),  # green
    (0, 216, 216, 255),  # cyan
    (0, 0, 216, 255),  # blue
    (216, 0, 216, 255),  # magenta
]


def draw_icon(size: int) -> Image.Image:
    """Draw the Spec Chum mark at ``size``×``size`` (RGBA)."""
    im = Image.new("RGBA", (size, size), BLACK)
    draw = ImageDraw.Draw(im)

    def u(v: float) -> int:
        return int(round(v * size / 256.0))

    margin = u(12)
    draw.rounded_rectangle(
        [margin, margin, size - 1 - margin, size - 1 - margin],
        radius=u(28),
        fill=BEZEL,
    )

    inset = u(36)
    stripe_h = u(22)
    draw.rounded_rectangle(
        [inset, inset, size - 1 - inset, size - 1 - inset - stripe_h - u(6)],
        radius=u(10),
        fill=SCREEN,
    )

    stripe_top = size - 1 - inset - stripe_h
    stripe_bot = size - 1 - inset
    band_w = (size - 2 * inset) / 6.0
    for i, color in enumerate(RAINBOW):
        x0 = inset + int(round(i * band_w))
        x1 = inset + int(round((i + 1) * band_w)) - 1
        draw.rectangle([x0, stripe_top, x1, stripe_bot], fill=color)

    if size >= 32:
        cw, ch = max(1, u(18)), max(1, u(28))
        cx = size // 2 - cw // 2
        cy = inset + u(48)
        draw.rectangle([cx, cy, cx + cw, cy + ch], fill=CURSOR)
    elif size >= 16:
        draw.rectangle(
            [size // 2 - 2, size // 2 - 4, size // 2 + 2, size // 2 + 4],
            fill=CURSOR,
        )

    if size >= 64:
        r = max(1, u(3))
        for sx, sy in (
            (u(22), u(22)),
            (size - u(22), u(22)),
            (u(22), size - u(22)),
            (size - u(22), size - u(22)),
        ):
            draw.ellipse([sx - r, sy - r, sx + r, sy + r], fill=WHITE)

    return im


def write_png(path: Path, size: int, *, rgba: bool) -> None:
    im = draw_icon(size)
    path.parent.mkdir(parents=True, exist_ok=True)
    if rgba:
        im.save(path, optimize=True)
    else:
        im.convert("RGB").save(path, optimize=True)
    print(f"wrote {path.relative_to(ROOT)} ({size}x{size})")


def write_ico(path: Path) -> None:
    sizes = [16, 24, 32, 48, 64, 128, 256]
    images = [draw_icon(sz) for sz in sizes]
    path.parent.mkdir(parents=True, exist_ok=True)
    images[-1].save(
        path,
        format="ICO",
        sizes=[(im.width, im.height) for im in images],
        append_images=images[:-1],
    )
    print(f"wrote {path.relative_to(ROOT)} ({len(sizes)} sizes)")


def write_icns(path: Path) -> None:
    if sys.platform != "darwin":
        print("skip AppIcon.icns (iconutil is macOS-only)", file=sys.stderr)
        if not path.is_file():
            raise SystemExit(
                "packaging/macos/AppIcon.icns missing; generate on macOS once"
            )
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
            draw_icon(sz).save(iconset / name)
        path.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(path)],
            check=True,
        )
    print(f"wrote {path.relative_to(ROOT)}")


def main() -> int:
    write_png(ROOT / "packaging/icon/spec-chum-256.png", 256, rgba=False)
    write_png(ROOT / "packaging/icon/spec-chum-512.png", 512, rgba=False)
    write_png(ROOT / "packaging/icon/spec-chum-1024.png", 1024, rgba=False)
    write_png(ROOT / "packaging/linux/spec-chum.png", 256, rgba=False)
    # RGBA for egui IconData (png crate decodes without expansion).
    write_png(ROOT / "crates/app/assets/icon.png", 256, rgba=True)
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
