import hashlib
import tempfile
import unittest
import zipfile
from pathlib import Path

from scripts import system_next_assets as assets


class SystemNextAssetTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.asset_dir = self.root / "assets"
        self.asset_dir.mkdir()

    def tearDown(self):
        self.temporary.cleanup()

    @staticmethod
    def digest(data):
        return hashlib.sha256(data).hexdigest()

    def make_asset_set(self):
        files = {"LICENSE.md": b"distribution notice"}
        archive_path = self.asset_dir / "system.zip"
        with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED) as archive:
            for name, data in files.items():
                archive.writestr(name, data)
        manifest = assets.ArchiveManifest(
            filename=archive_path.name,
            size=archive_path.stat().st_size,
            sha256=assets._sha256(archive_path),
            url="https://example.invalid/system.zip",
            members={"LICENSE.md": (len(files["LICENSE.md"]), self.digest(files["LICENSE.md"]))},
        )
        boot_rom = b"pinned boot rom"
        license_text = b"GPL3 license text"
        (self.asset_dir / "boot.bin").write_bytes(boot_rom)
        (self.asset_dir / "GPL3-LICENSE").write_bytes(license_text)
        (self.asset_dir / "ASSET-INFO.txt").write_text("pinned source metadata\n")
        return manifest, boot_rom, license_text

    def verify(self, manifest, boot_rom, license_text):
        assets.verify_installed_assets(
            self.asset_dir,
            system_manifest=manifest,
            boot_name="boot.bin",
            boot_size=len(boot_rom),
            boot_hash=self.digest(boot_rom),
            license_size=len(license_text),
            license_hash=self.digest(license_text),
            metadata="pinned source metadata\n",
        )

    def test_missing_asset_fails_without_network(self):
        manifest, boot_rom, license_text = self.make_asset_set()
        (self.asset_dir / "boot.bin").unlink()
        with self.assertRaisesRegex(assets.AssetError, "missing required asset"):
            self.verify(manifest, boot_rom, license_text)

    def test_mismatched_asset_fails_without_network(self):
        manifest, boot_rom, license_text = self.make_asset_set()
        (self.asset_dir / "boot.bin").write_bytes(b"modified boot rom")
        with self.assertRaisesRegex(assets.AssetError, "size or SHA-256 mismatch"):
            self.verify(manifest, boot_rom, license_text)

    def test_missing_firmware_archive_member_names_the_file(self):
        archive_path = self.asset_dir / "system.zip"
        with zipfile.ZipFile(archive_path, "w") as archive:
            archive.writestr("LICENSE.md", b"distribution notice")
        manifest = assets.ArchiveManifest(
            filename=archive_path.name,
            size=archive_path.stat().st_size,
            sha256=assets._sha256(archive_path),
            url="https://example.invalid/system.zip",
            members={"TBBLUE.FW": (4, self.digest(b"firmware"))},
        )
        with self.assertRaisesRegex(assets.AssetError, "system.zip is missing TBBLUE.FW"):
            assets.verify_archive(archive_path, manifest)

    def test_corrupt_firmware_archive_member_is_reported(self):
        archive_path = self.asset_dir / "system.zip"
        with zipfile.ZipFile(archive_path, "w") as archive:
            archive.writestr("TBBLUE.FW", b"bad!")
        manifest = assets.ArchiveManifest(
            filename=archive_path.name,
            size=archive_path.stat().st_size,
            sha256=assets._sha256(archive_path),
            url="https://example.invalid/system.zip",
            members={"TBBLUE.FW": (8, self.digest(b"firmware"))},
        )
        with self.assertRaisesRegex(
            assets.AssetError, "system.zip member hash mismatch: TBBLUE.FW"
        ):
            assets.verify_archive(archive_path, manifest)

    def test_complete_pinned_asset_set_passes_without_network(self):
        manifest, boot_rom, license_text = self.make_asset_set()
        self.verify(manifest, boot_rom, license_text)


if __name__ == "__main__":
    unittest.main()
