#!/usr/bin/env python3
"""Build a temporary full-tree FAT16 card and run the opt-in Next boot test."""

from __future__ import annotations

import os
import plistlib
import shutil
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
ASSETS = Path(
    os.environ.get("SPEC_CHUM_NEXT_ASSET_DIR", ROOT / "roms" / "system-next" / "24.11")
).expanduser().resolve()
ARCHIVE = ASSETS / "sn-complete-24.11.zip"
MAIN_ROM_MEMBER = "machines/next/enNextZX.rom"
DIVMMC_ROM_MEMBER = "machines/next/enNxtmmc.rom"


def run(*args: str, cwd: Path = ROOT, capture: bool = False) -> str:
    result = subprocess.run(
        args,
        cwd=cwd,
        check=False,
        text=True,
        capture_output=capture,
    )
    if result.returncode:
        if capture:
            sys.stderr.write(result.stderr)
        raise SystemExit(result.returncode)
    return result.stdout if capture else ""


def attached_device(hdiutil: str, image: Path) -> str:
    info = plistlib.loads(run(hdiutil, "info", "-plist", capture=True).encode())
    image_path = image.resolve()
    for mounted_image in info.get("images", []):
        if Path(mounted_image.get("image-path", "")).resolve() != image_path:
            continue
        for entity in mounted_image.get("system-entities", []):
            device = entity.get("dev-entry", "")
            if device.startswith("/dev/disk") and device[len("/dev/disk") :].isdigit():
                return device
    raise SystemExit(f"could not find attached device for temporary image {image}")


def main() -> int:
    if sys.platform != "darwin":
        raise SystemExit("the full-card boot fixture currently requires macOS hdiutil")

    run(sys.executable, str(ROOT / "scripts" / "system_next_assets.py"), "--verify")
    hdiutil = shutil.which("hdiutil")
    if hdiutil is None:
        raise SystemExit("hdiutil is required to create the temporary FAT16 card")

    with tempfile.TemporaryDirectory(prefix="spec-chum-next-boot-") as temporary:
        temp = Path(temporary)
        sparse = temp / "full-tree.sparseimage"
        mount = temp / "mount"
        raw_base = temp / "full-tree.raw"
        error_raw_base = temp / "missing-file.raw"
        main_rom = temp / "enNextZX.rom"
        divmmc_rom = temp / "enNxtmmc.rom"
        mount.mkdir()

        run(
            hdiutil,
            "create",
            "-quiet",
            "-size",
            "256m",
            "-fs",
            "MS-DOS FAT16",
            "-volname",
            "SPECNEXT",
            "-layout",
            "MBRSPUD",
            "-type",
            "SPARSE",
            str(sparse),
        )
        attached = False
        device = ""
        try:
            plist_output = run(
                hdiutil,
                "attach",
                "-plist",
                "-nobrowse",
                "-noautoopen",
                "-mountpoint",
                str(mount),
                str(sparse),
                capture=True,
            )
            attached = True
            entities = plistlib.loads(plist_output.encode())["system-entities"]
            partition = next(
                (
                    item
                    for item in entities
                    if item.get("dev-entry", "").endswith("s1")
                ),
                None,
            )
            if partition is None:
                raise SystemExit("hdiutil attached the card but returned no partition device")
            device = partition["dev-entry"].rsplit("s", maxsplit=1)[0]
            entity = next(
                (
                    item
                    for item in entities
                    if item.get("mount-point")
                    and Path(item["mount-point"]).resolve() == mount.resolve()
                ),
                None,
            )
            if entity is None:
                raise SystemExit("hdiutil attached the card but returned no matching mount")

            with zipfile.ZipFile(ARCHIVE) as archive:
                archive.extractall(mount)
                main_rom.write_bytes(archive.read(MAIN_ROM_MEMBER))
                divmmc_rom.write_bytes(archive.read(DIVMMC_ROM_MEMBER))
            (mount / "machines" / "next" / "CONFIG.INI").write_text(
                "timing=7\n", encoding="ascii"
            )
        finally:
            if attached:
                if not device:
                    device = attached_device(hdiutil, sparse)
                run(hdiutil, "detach", device)

        run(
            hdiutil,
            "convert",
            "-quiet",
            "-format",
            "UDRW",
            "-o",
            str(raw_base),
            str(sparse),
        )
        raw_image = Path(f"{raw_base}.dmg")
        if not raw_image.is_file():
            raise SystemExit(f"hdiutil did not produce the raw card image: {raw_image}")

        attached = False
        device = ""
        try:
            plist_output = run(
                hdiutil,
                "attach",
                "-plist",
                "-nobrowse",
                "-noautoopen",
                "-mountpoint",
                str(mount),
                str(sparse),
                capture=True,
            )
            attached = True
            entities = plistlib.loads(plist_output.encode())["system-entities"]
            partition = next(
                (item for item in entities if item.get("dev-entry", "").endswith("s1")),
                None,
            )
            if partition is None:
                raise SystemExit("hdiutil attached the card but returned no partition device")
            device = partition["dev-entry"].rsplit("s", maxsplit=1)[0]
            (mount / "machines" / "next" / "menu.def").unlink()
        finally:
            if attached:
                if not device:
                    device = attached_device(hdiutil, sparse)
                run(hdiutil, "detach", device)

        run(
            hdiutil,
            "convert",
            "-quiet",
            "-format",
            "UDRW",
            "-o",
            str(error_raw_base),
            str(sparse),
        )
        error_raw_image = Path(f"{error_raw_base}.dmg")
        if not error_raw_image.is_file():
            raise SystemExit(
                f"hdiutil did not produce the missing-file card image: {error_raw_image}"
            )

        environment = os.environ.copy()
        environment["SPEC_CHUM_NEXT_BOOT_CARD"] = str(raw_image)
        environment["SPEC_CHUM_NEXT_BOOT_ERROR_CARD"] = str(error_raw_image)
        environment["SPEC_CHUM_NEXT_BOOT_ROM"] = str(main_rom)
        environment["SPEC_CHUM_NEXT_BOOT_DIVMMC_ROM"] = str(divmmc_rom)
        result = subprocess.run(
            [
                "cargo",
                "test",
                "-p",
                "machine",
                "--features",
                "system-tests",
                "--release",
                "next_boot_system_tests",
                "--",
                "--nocapture",
            ],
            cwd=ROOT,
            env=environment,
            check=False,
        )
        if result.returncode:
            return result.returncode
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
