#!/usr/bin/env python3
"""Fetch or verify the pinned, user-provided Spectrum Next boot assets."""

from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import subprocess
import sys
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping, Optional


ROOT = Path(__file__).resolve().parent.parent
ASSET_DIR = Path(
    os.environ.get(
        "SPEC_CHUM_NEXT_ASSET_DIR", ROOT / "roms" / "system-next" / "24.11"
    )
)
CACHE_DIR = Path(
    os.environ.get(
        "SPEC_CHUM_NEXT_CACHE_DIR", ROOT / ".rom-cache" / "system-next"
    )
)

SYSTEM_ARCHIVE = "sn-complete-24.11.zip"
SYSTEM_URL = "https://www.specnext.com/distro/24.11/sn-complete-24.11.zip"
SYSTEM_SHA256 = "cbf5d4c8bb6dc552a4e68317a7315e06609b14028e1f04dd0afd2189be65ce6b"
SYSTEM_SIZE = 56_371_963
SYSTEM_MEMBERS = {
    "LICENSE.md": (5_186, "48a5fdf32c0d9ea9d7ba44afca1b28d487bad39ae0aa1f0ee426b291cb4dc496"),
    "docs/licenses/LICENSE": (1_170, "7cd1e84c1ed05c2035d0bcdc1b168d4d124c831e2d24c6327a044d0ed2fedbad"),
    "TBBLUE.FW": (304_640, "8bb1c9dd0a747560decf63aa56e16fde70dcd9b9f9eda7863b65a062405fb580"),
    "machines/next/enNextZX.rom": (
        65_536,
        "9419872e54d19285661be451ef0fb86195b5d6c8f9f0be6d03559ec831884317",
    ),
    "machines/next/enNxtmmc.rom": (
        8_192,
        "36d35b62635776dcd8ae462bb45102094873f324b99b7982a5fb6a5c05b3a8f6",
    ),
    "machines/next/enNextMf.rom": (
        8_192,
        "0ee4ba5f830c146b878a316ebabff893e12d6c2ceda85f90883ae67647c0698e",
    ),
    "machines/next/menu.def": (
        750,
        "f41ee1dbb8387af36ee372e31487b363a9d00d973f5ea39a6f3faa88c673624b",
    ),
    "docs/licenses/BBCBasic/COPYING": (
        856,
        "cf5efb79a693ab044d2c5354d00f682e22fde66b428da1b4dc24cb1ad2ef42bb",
    ),
}

IPL_ARCHIVE = "tbblue.zip"
IPL_URL = "https://www.specnext.com/forum/download/file.php?id=1164"
IPL_SHA256 = "845b6567cbb531a550aff6e762bf0b6aa5ea2855bd23be24925ed6202fdc3d03"
IPL_SIZE = 45_246
IPL_ROM = "boot-30204.bin"
IPL_ROM_SIZE = 8_192
IPL_ROM_SHA256 = "33f04fd104eb428eff1afe18854e3fc232019a20948ce8efed01f04f1196d815"
GPL3_FILE = "GPL3-LICENSE"
GPL3_SIZE = 35_176
GPL3_SHA256 = "9ec6baf9712f086f8047c71ffe78e265bc8caac8ef2e924a421c7bf261de28b0"
IPL_README = "README.md"
IPL_README_SIZE = 2_609


class AssetError(Exception):
    """An asset is missing, corrupt, or not the pinned official version."""


@dataclass(frozen=True)
class ArchiveManifest:
    filename: str
    size: int
    sha256: str
    url: str
    members: Mapping[str, tuple[int, str]]


SYSTEM_MANIFEST = ArchiveManifest(
    SYSTEM_ARCHIVE, SYSTEM_SIZE, SYSTEM_SHA256, SYSTEM_URL, SYSTEM_MEMBERS
)


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _member_sha256(archive: zipfile.ZipFile, member: str) -> tuple[int, str]:
    digest = hashlib.sha256()
    size = 0
    with archive.open(member) as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            size += len(block)
            digest.update(block)
    return size, digest.hexdigest()


def verify_archive(path: Path, manifest: ArchiveManifest) -> None:
    if not path.is_file():
        raise AssetError(f"missing {manifest.filename}: {path}")
    if path.stat().st_size != manifest.size:
        raise AssetError(
            f"{manifest.filename} has {path.stat().st_size} bytes; "
            f"expected {manifest.size}"
        )
    actual_hash = _sha256(path)
    if actual_hash != manifest.sha256:
        raise AssetError(
            f"{manifest.filename} SHA-256 mismatch: {actual_hash}; "
            f"expected {manifest.sha256}"
        )

    try:
        with zipfile.ZipFile(path) as archive:
            bad_member = archive.testzip()
            if bad_member is not None:
                raise AssetError(f"{manifest.filename} has corrupt member {bad_member}")
            names = set(archive.namelist())
            for member, expected in manifest.members.items():
                if member not in names:
                    raise AssetError(f"{manifest.filename} is missing {member}")
                if _member_sha256(archive, member) != expected:
                    raise AssetError(f"{manifest.filename} member hash mismatch: {member}")
    except zipfile.BadZipFile as error:
        raise AssetError(f"{manifest.filename} is not a valid ZIP archive") from error


def verify_ipl_archive(path: Path) -> None:
    manifest = ArchiveManifest(
        IPL_ARCHIVE,
        IPL_SIZE,
        IPL_SHA256,
        IPL_URL,
        {
            IPL_ROM: (IPL_ROM_SIZE, IPL_ROM_SHA256),
            GPL3_FILE: (GPL3_SIZE, GPL3_SHA256),
            IPL_README: (IPL_README_SIZE, ""),
        },
    )
    if not path.is_file():
        raise AssetError(f"missing {IPL_ARCHIVE}: {path}")
    if path.stat().st_size != manifest.size:
        raise AssetError(
            f"{IPL_ARCHIVE} has {path.stat().st_size} bytes; expected {manifest.size}"
        )
    if _sha256(path) != manifest.sha256:
        raise AssetError(f"{IPL_ARCHIVE} SHA-256 mismatch; expected {manifest.sha256}")
    try:
        with zipfile.ZipFile(path) as archive:
            bad_member = archive.testzip()
            if bad_member is not None:
                raise AssetError(f"{IPL_ARCHIVE} has corrupt member {bad_member}")
            names = set(archive.namelist())
            for member, (expected_size, expected_hash) in manifest.members.items():
                if member not in names:
                    raise AssetError(f"{IPL_ARCHIVE} is missing {member}")
                if expected_hash and _member_sha256(archive, member) != (
                    expected_size,
                    expected_hash,
                ):
                    raise AssetError(f"{IPL_ARCHIVE} member hash mismatch: {member}")
            readme = archive.read(IPL_README).decode("utf-8")
            if "License applied to boot code versions 30204 and later is GPL3" not in readme:
                raise AssetError(f"{IPL_ARCHIVE} does not document the GPL3 boot code")
    except zipfile.BadZipFile as error:
        raise AssetError(f"{IPL_ARCHIVE} is not a valid ZIP archive") from error


def _download(url: str, destination: Path, expected_hash: str) -> None:
    if destination.exists():
        if _sha256(destination) == expected_hash:
            return
        raise AssetError(
            f"cached {destination.name} does not match its pinned SHA-256; "
            "remove the bad cache file and retry"
        )

    temporary = destination.with_suffix(destination.suffix + ".part")
    try:
        result = subprocess.run(
            [
                "curl",
                "--fail",
                "--location",
                "--retry",
                "2",
                "--connect-timeout",
                "15",
                "--max-time",
                "120",
                "--output",
                str(temporary),
                url,
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise AssetError(
                f"could not download {destination.name}: "
                f"{result.stderr.strip() or result.returncode}"
            )
        actual_hash = _sha256(temporary)
        if actual_hash != expected_hash:
            raise AssetError(
                f"downloaded {destination.name} SHA-256 mismatch: {actual_hash}; "
                f"expected {expected_hash}"
            )
        temporary.replace(destination)
    except OSError as error:
        temporary.unlink(missing_ok=True)
        raise AssetError(f"could not download {destination.name}: {error}") from error
    except AssetError:
        temporary.unlink(missing_ok=True)
        raise


def _copy_exact(source: Path, destination: Path, expected_hash: str) -> None:
    if destination.exists():
        if _sha256(destination) == expected_hash:
            return
        raise AssetError(
            f"installed {destination.name} differs from the pinned asset; "
            "remove it before retrying"
        )
    temporary = destination.with_suffix(destination.suffix + ".part")
    shutil.copyfile(source, temporary)
    if _sha256(temporary) != expected_hash:
        temporary.unlink(missing_ok=True)
        raise AssetError(f"copy verification failed for {destination.name}")
    temporary.replace(destination)


def verify_installed_assets(
    asset_dir: Path,
    system_manifest: ArchiveManifest = SYSTEM_MANIFEST,
    boot_name: str = IPL_ROM,
    boot_size: int = IPL_ROM_SIZE,
    boot_hash: str = IPL_ROM_SHA256,
    license_name: str = GPL3_FILE,
    license_size: int = GPL3_SIZE,
    license_hash: str = GPL3_SHA256,
    metadata: Optional[str] = None,
) -> None:
    verify_archive(asset_dir / system_manifest.filename, system_manifest)
    verify_file(asset_dir / boot_name, boot_size, boot_hash)
    verify_file(asset_dir / license_name, license_size, license_hash)
    metadata_path = asset_dir / "ASSET-INFO.txt"
    expected_metadata = _asset_metadata() if metadata is None else metadata
    if not metadata_path.is_file() or metadata_path.read_text() != expected_metadata:
        raise AssetError("missing or mismatched ASSET-INFO.txt")


def verify_file(path: Path, expected_size: int, expected_hash: str) -> None:
    if not path.is_file():
        raise AssetError(f"missing required asset: {path.name}")
    if path.stat().st_size != expected_size or _sha256(path) != expected_hash:
        raise AssetError(f"{path.name} size or SHA-256 mismatch")


def _asset_metadata() -> str:
    return (
        "Source: official SpecNext System/Next Distribution 24.11\n"
        f"Archive: {SYSTEM_URL}\nSHA-256: {SYSTEM_SHA256}\n"
        f"IPL source: {IPL_URL}\nIPL archive SHA-256: {IPL_SHA256}\n"
        f"Selected IPL: {IPL_ROM}\nIPL SHA-256: {IPL_ROM_SHA256}\n"
        "IPL license: GPL-3.0-or-later (GPL3-LICENSE included)\n"
    )


def install_assets() -> None:
    CACHE_DIR.mkdir(parents=True, exist_ok=True)
    ASSET_DIR.mkdir(parents=True, exist_ok=True)
    system_cache = CACHE_DIR / SYSTEM_ARCHIVE
    ipl_cache = CACHE_DIR / IPL_ARCHIVE
    _download(SYSTEM_URL, system_cache, SYSTEM_SHA256)
    verify_archive(system_cache, SYSTEM_MANIFEST)
    _download(IPL_URL, ipl_cache, IPL_SHA256)
    verify_ipl_archive(ipl_cache)

    _copy_exact(system_cache, ASSET_DIR / SYSTEM_ARCHIVE, SYSTEM_SHA256)
    with zipfile.ZipFile(ipl_cache) as archive:
        for member, expected_hash in ((IPL_ROM, IPL_ROM_SHA256), (GPL3_FILE, GPL3_SHA256)):
            destination = ASSET_DIR / member
            if destination.exists():
                verify_file(
                    destination,
                    IPL_ROM_SIZE if member == IPL_ROM else GPL3_SIZE,
                    expected_hash,
                )
                continue
            temporary = destination.with_suffix(destination.suffix + ".part")
            with archive.open(member) as source, temporary.open("wb") as out:
                shutil.copyfileobj(source, out)
            if _sha256(temporary) != expected_hash:
                temporary.unlink(missing_ok=True)
                raise AssetError(f"extracted {member} failed SHA-256 validation")
            temporary.replace(destination)

    metadata = _asset_metadata()
    metadata_path = ASSET_DIR / "ASSET-INFO.txt"
    if metadata_path.exists() and metadata_path.read_text() != metadata:
        raise AssetError("ASSET-INFO.txt differs from the pinned asset metadata")
    metadata_path.write_text(metadata)
    verify_installed_assets(ASSET_DIR)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--verify",
        action="store_true",
        help="verify installed assets without downloading anything",
    )
    args = parser.parse_args()
    try:
        if args.verify:
            verify_installed_assets(ASSET_DIR)
        else:
            install_assets()
    except AssetError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"Spectrum Next assets are valid: {ASSET_DIR}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
