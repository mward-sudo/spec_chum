//! User-owned Spectrum Next assets and the runtime SD card derived from them.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use sha2::{Digest, Sha256};
use thiserror::Error;
use zip::ZipArchive;

const VERSION_DIR: &str = "roms/system-next/24.11";
const SYSTEM_ARCHIVE: &str = "sn-complete-24.11.zip";
const SYSTEM_SIZE: u64 = 56_371_963;
const SYSTEM_SHA256: &str = "cbf5d4c8bb6dc552a4e68317a7315e06609b14028e1f04dd0afd2189be65ce6b";
const SYSTEM_URL: &str = "https://www.specnext.com/distro/24.11/sn-complete-24.11.zip";
const IPL_NAME: &str = "boot-30204.bin";
const IPL_SHA256: &str = "33f04fd104eb428eff1afe18854e3fc232019a20948ce8efed01f04f1196d815";
const IPL_ARCHIVE: &str = "tbblue.zip";
const IPL_ARCHIVE_URL: &str = "https://www.specnext.com/forum/download/file.php?id=1164";
const IPL_ARCHIVE_SHA256: &str = "845b6567cbb531a550aff6e762bf0b6aa5ea2855bd23be24925ed6202fdc3d03";
const IPL_ARCHIVE_BYTES: u64 = 45_246;
const LICENSE_NAME: &str = "GPL3-LICENSE";
const LICENSE_SHA256: &str = "9ec6baf9712f086f8047c71ffe78e265bc8caac8ef2e924a421c7bf261de28b0";
const CARD_NAME: &str = "next-card.img";
const CARD_MARKER: &str = "next-card.source";
const CARD_BYTES: u64 = 256 * 1024 * 1024;
const PARTITION_LBA: u32 = 2048;
const SECTOR_BYTES: u64 = 512;
const ASSET_INFO: &str = concat!(
    "Source: official SpecNext System/Next Distribution 24.11\n",
    "Archive: https://www.specnext.com/distro/24.11/sn-complete-24.11.zip\n",
    "SHA-256: cbf5d4c8bb6dc552a4e68317a7315e06609b14028e1f04dd0afd2189be65ce6b\n",
    "IPL source: https://www.specnext.com/forum/download/file.php?id=1164\n",
    "IPL archive SHA-256: 845b6567cbb531a550aff6e762bf0b6aa5ea2855bd23be24925ed6202fdc3d03\n",
    "Selected IPL: boot-30204.bin\n",
    "IPL SHA-256: 33f04fd104eb428eff1afe18854e3fc232019a20948ce8efed01f04f1196d815\n",
    "IPL license: GPL-3.0-or-later (GPL3-LICENSE included)\n",
    "IPL upstream source reference (from archive README): https://gitlab.com/SpectrumNext/ZX_Spectrum_Next_FPGA/-/tree/master/cores/zxnext/src/rom?ref_type=heads\n",
    "Exact source correspondence for boot-30204.bin is unverified.\n",
);
type AssetStamp = [Option<(u64, SystemTime)>; 4];
static AVAILABILITY: Mutex<Option<(PathBuf, AssetStamp, bool)>> = Mutex::new(None);

fn card_directory(asset_directory: &Path) -> PathBuf {
    if let Some(directory) = std::env::var_os("SPEC_CHUM_NEXT_CARD_DIR") {
        return PathBuf::from(directory);
    }
    directories::ProjectDirs::from("dev", "SpecChum", "spec-chum").map_or_else(
        || asset_directory.to_path_buf(),
        |dirs| dirs.data_dir().join("system-next/24.11"),
    )
}

/// User action required before selecting Next; this guidance never initiates a download.
pub const SETUP_GUIDANCE: &str = "Select Get official assets in ROM Setup, or select a previously verified sn-complete-24.11.zip with its companion IPL and notices. The first launch prepares a local SD card from the verified archive. See https://github.com/mward-sudo/spec_chum/blob/main/docs/ROMS.md.";

#[derive(Debug, Error)]
pub enum NextAssetError {
    #[error("{0}. {SETUP_GUIDANCE}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error("official asset download failed: {0}")]
    Network(String),
}

#[derive(Clone, Debug)]
pub struct NextAssets {
    pub directory: PathBuf,
    pub ipl: PathBuf,
    pub archive: PathBuf,
    pub card: PathBuf,
}

fn digest(path: &Path) -> io::Result<String> {
    let mut source = File::open(path)?;
    let mut hash = Sha256::new();
    io::copy(&mut source, &mut hash_writer(&mut hash))?;
    Ok(format!("{:x}", hash.finalize()))
}

fn hash_writer(hash: &mut Sha256) -> impl Write + '_ {
    struct Writer<'a>(&'a mut Sha256);
    impl Write for Writer<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    Writer(hash)
}

fn verify_file(path: &Path, size: u64, sha256: &str) -> Result<(), NextAssetError> {
    let actual = fs::metadata(path)
        .map_err(|_| NextAssetError::Invalid(format!("missing {}", path.display())))?;
    if actual.len() != size || digest(path)? != sha256 {
        return Err(NextAssetError::Invalid(format!(
            "{} does not match the pinned official asset",
            path.display()
        )));
    }
    Ok(())
}

fn download_verified(
    url: &str,
    destination: &Path,
    size: u64,
    sha256: &str,
) -> Result<(), NextAssetError> {
    let temporary = destination.with_extension("download");
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(120)))
        .user_agent("Spec Chum Spectrum Next asset setup")
        .build()
        .new_agent();
    let response = agent
        .get(url)
        .call()
        .map_err(|error| NextAssetError::Network(error.to_string()))?;
    let mut source = response.into_body().into_reader().take(size + 1);
    let mut file = File::create(&temporary)?;
    let copied = io::copy(&mut source, &mut file)?;
    drop(file);
    if copied != size || digest(&temporary)? != sha256 {
        fs::remove_file(temporary)?;
        return Err(NextAssetError::Invalid(format!(
            "downloaded {url} differs from the pinned official asset"
        )));
    }
    fs::rename(temporary, destination)?;
    Ok(())
}

fn extract_verified_member<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    directory: &Path,
    name: &str,
    size: u64,
    sha256: &str,
) -> Result<(), NextAssetError> {
    let mut member = archive.by_name(name)?;
    let target = directory.join(name);
    let temporary = target.with_extension("verified-part");
    let extraction = (|| {
        let mut file = File::create(&temporary)?;
        io::copy(&mut member, &mut file)?;
        file.sync_all()?;
        drop(file);
        verify_file(&temporary, size, sha256)
    })();
    if let Err(error) = extraction {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if target.exists() {
        fs::remove_file(&target)?;
    }
    fs::rename(temporary, target)?;
    Ok(())
}

impl NextAssets {
    /// Availability for an explicit source directory; the same validation gates boot.
    pub(crate) fn available_in(directory: &Path) -> bool {
        Self::discover_in(directory.to_path_buf()).is_ok()
    }

    /// Fetch official assets after an explicit user action, preserving the complete
    /// distribution archive and the separate GPL IPL licence in per-user storage.
    pub fn acquire_official() -> Result<Self, NextAssetError> {
        let directory = card_directory(Path::new(VERSION_DIR));
        fs::create_dir_all(&directory)?;
        let archive = directory.join(SYSTEM_ARCHIVE);
        if !archive.exists() {
            let packaged = machine::search_roots()
                .into_iter()
                .map(|root| root.join(VERSION_DIR).join(SYSTEM_ARCHIVE))
                .find(|path| path.is_file());
            if let Some(source) = packaged {
                verify_file(&source, SYSTEM_SIZE, SYSTEM_SHA256)?;
                fs::copy(source, &archive)?;
            } else {
                download_verified(SYSTEM_URL, &archive, SYSTEM_SIZE, SYSTEM_SHA256)?;
            }
        }
        verify_file(&archive, SYSTEM_SIZE, SYSTEM_SHA256)?;

        let ipl = directory.join(IPL_NAME);
        let license = directory.join(LICENSE_NAME);
        if !ipl.exists() || !license.exists() {
            let temporary = directory.join(IPL_ARCHIVE);
            if !temporary.exists() {
                download_verified(
                    IPL_ARCHIVE_URL,
                    &temporary,
                    IPL_ARCHIVE_BYTES,
                    IPL_ARCHIVE_SHA256,
                )?;
            }
            verify_file(&temporary, IPL_ARCHIVE_BYTES, IPL_ARCHIVE_SHA256)?;
            let mut source = ZipArchive::new(File::open(&temporary)?)?;
            for (name, size, sha256) in [
                (IPL_NAME, 8192, IPL_SHA256),
                (LICENSE_NAME, 35_176, LICENSE_SHA256),
            ] {
                extract_verified_member(&mut source, &directory, name, size, sha256)?;
            }
        }
        fs::write(directory.join("ASSET-INFO.txt"), ASSET_INFO)?;
        Self::discover_in(directory)
    }

    fn candidate_directory() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("SPEC_CHUM_NEXT_ASSET_DIR") {
            return Some(PathBuf::from(path));
        }
        let key =
            crate::prefs::model_rom_path_key(crate::prefs::PrefModel::SpectrumNext, "next_assets");
        if let Some(path) = crate::rom_setup::model_rom_paths_snapshot().get(&key) {
            return Path::new(path).parent().map(Path::to_path_buf);
        }
        let user = card_directory(Path::new(VERSION_DIR));
        if user.join(SYSTEM_ARCHIVE).is_file() {
            return Some(user);
        }
        machine::search_roots()
            .into_iter()
            .map(|root| root.join(VERSION_DIR))
            .find(|path| path.join(SYSTEM_ARCHIVE).is_file())
    }

    /// Cheap UI availability probe; revalidates when any pinned file changes.
    #[must_use]
    pub fn available_cached() -> bool {
        let Some(directory) = Self::candidate_directory() else {
            return false;
        };
        let stamp = [SYSTEM_ARCHIVE, IPL_NAME, LICENSE_NAME, "ASSET-INFO.txt"].map(|name| {
            fs::metadata(directory.join(name))
                .ok()
                .and_then(|meta| meta.modified().ok().map(|modified| (meta.len(), modified)))
        });
        if let Ok(cache) = AVAILABILITY.lock() {
            if let Some((path, previous, available)) = cache.as_ref() {
                if *path == directory && *previous == stamp {
                    return *available;
                }
            }
        }
        let available = Self::available_in(&directory);
        if let Ok(mut cache) = AVAILABILITY.lock() {
            *cache = Some((directory, stamp, available));
        }
        available
    }

    /// Locate and verify the complete user-installed asset set from #525.
    pub fn discover() -> Result<Self, NextAssetError> {
        let directory = Self::candidate_directory().ok_or_else(|| {
            NextAssetError::Invalid("official System/Next archive is missing".into())
        })?;
        Self::discover_in(directory)
    }

    /// Validate all required files in a directory selected by a host setup dialog.
    pub fn discover_in(directory: PathBuf) -> Result<Self, NextAssetError> {
        let archive = directory.join(SYSTEM_ARCHIVE);
        let ipl = directory.join(IPL_NAME);
        verify_file(&archive, SYSTEM_SIZE, SYSTEM_SHA256)?;
        verify_file(&ipl, 8192, IPL_SHA256)?;
        verify_file(&directory.join(LICENSE_NAME), 35_176, LICENSE_SHA256)?;
        let metadata = fs::read_to_string(directory.join("ASSET-INFO.txt")).map_err(|_| {
            NextAssetError::Invalid("asset source and license notices are missing".into())
        })?;
        if metadata != ASSET_INFO {
            return Err(NextAssetError::Invalid(
                "asset source and license notices differ".into(),
            ));
        }
        let card = card_directory(&directory).join(CARD_NAME);
        Ok(Self {
            directory,
            ipl,
            archive,
            card,
        })
    }

    /// Prepare a persistent local card once; subsequent boots retain guest writes.
    pub fn prepare_card(&self) -> Result<&Path, NextAssetError> {
        let parent = self
            .card
            .parent()
            .ok_or_else(|| NextAssetError::Invalid("SD card path has no directory".into()))?;
        fs::create_dir_all(parent)?;
        let marker = parent.join(CARD_MARKER);
        if self.card.is_file()
            && fs::metadata(&self.card)?.len() == CARD_BYTES
            && fs::read_to_string(&marker).ok().as_deref() == Some(SYSTEM_SHA256)
        {
            return Ok(&self.card);
        }
        if self.card.exists() {
            return Err(NextAssetError::Invalid(format!(
                "existing Next SD card {} has an unexpected size or source marker; move it aside after backing up guest data",
                self.card.display()
            )));
        }
        let temporary = parent.join(format!("{CARD_NAME}.part"));
        if temporary.exists() {
            fs::remove_file(&temporary)?;
        }
        let result = self.write_card(&temporary);
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        fs::write(marker, SYSTEM_SHA256)?;
        fs::rename(temporary, &self.card)?;
        Ok(&self.card)
    }

    fn write_card(&self, destination: &Path) -> Result<(), NextAssetError> {
        let mut card = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(destination)?;
        card.set_len(CARD_BYTES)?;
        let mut mbr = [0u8; 512];
        mbr[446] = 0x80;
        mbr[450] = 0x06; // FAT16 partition.
        mbr[454..458].copy_from_slice(&PARTITION_LBA.to_le_bytes());
        let sectors = (CARD_BYTES / SECTOR_BYTES) as u32 - PARTITION_LBA;
        mbr[458..462].copy_from_slice(&sectors.to_le_bytes());
        mbr[510..512].copy_from_slice(&[0x55, 0xaa]);
        card.write_all(&mbr)?;
        let partition = Partition::new(
            card,
            PARTITION_LBA as u64 * SECTOR_BYTES,
            sectors as u64 * SECTOR_BYTES,
        );
        fatfs::format_volume(
            partition,
            FormatVolumeOptions::new()
                .fat_type(FatType::Fat16)
                .volume_label(*b"SPECNEXT   "),
        )?;
        let card = OpenOptions::new()
            .read(true)
            .write(true)
            .open(destination)?;
        let partition = Partition::new(
            card,
            PARTITION_LBA as u64 * SECTOR_BYTES,
            sectors as u64 * SECTOR_BYTES,
        );
        let fs = FileSystem::new(partition, FsOptions::new())?;
        let root = fs.root_dir();
        let mut archive = ZipArchive::new(File::open(&self.archive)?)?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let name = entry.name().replace('\\', "/");
            let parts: Vec<&str> = name.split('/').filter(|part| !part.is_empty()).collect();
            if parts.iter().any(|part| *part == "." || *part == "..") {
                return Err(NextAssetError::Invalid(format!(
                    "unsafe archive entry {name}"
                )));
            }
            if parts.is_empty() {
                continue;
            }
            let mut path = String::new();
            for part in &parts[..parts.len() - 1] {
                if !path.is_empty() {
                    path.push('/');
                }
                path.push_str(part);
                root.create_dir(&path)?;
            }
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(parts[parts.len() - 1]);
            if entry.is_dir() {
                root.create_dir(&path)?;
            } else {
                let mut file = root.create_file(&path)?;
                io::copy(&mut entry, &mut file)?;
            }
        }
        root.create_dir("machines/next")?;
        root.create_file("machines/next/CONFIG.INI")?
            .write_all(b"timing=7\n")?;
        drop(root);
        fs.unmount()?;
        Ok(())
    }
}

/// Present a partition as a zero-based disk to `fatfs`.
struct Partition {
    file: File,
    start: u64,
    length: u64,
    position: u64,
}

impl Partition {
    fn new(file: File, start: u64, length: u64) -> Self {
        Self {
            file,
            start,
            length,
            position: 0,
        }
    }
}

impl Read for Partition {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let remaining = self.length.saturating_sub(self.position);
        self.file
            .seek(SeekFrom::Start(self.start + self.position))?;
        let limit = bytes.len().min(remaining as usize);
        let count = self.file.read(&mut bytes[..limit])?;
        self.position += count as u64;
        Ok(count)
    }
}

impl Write for Partition {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let remaining = self.length.saturating_sub(self.position);
        if bytes.len() as u64 > remaining {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "partition full"));
        }
        self.file
            .seek(SeekFrom::Start(self.start + self.position))?;
        let count = self.file.write(bytes)?;
        self.position += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for Partition {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let next = match pos {
            SeekFrom::Start(value) => value as i128,
            SeekFrom::Current(value) => self.position as i128 + value as i128,
            SeekFrom::End(value) => self.length as i128 + value as i128,
        };
        if next < 0 || next > self.length as i128 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "partition seek out of range",
            ));
        }
        self.position = next as u64;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use zip::write::SimpleFileOptions;

    #[test]
    fn invalid_extracted_asset_does_not_replace_existing_file() {
        let directory = std::env::temp_dir().join(format!(
            "spec-chum-next-asset-extract-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock follows Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create isolated asset directory");
        let destination = directory.join(IPL_NAME);
        fs::write(&destination, b"user file").expect("write existing asset");

        let mut bytes = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut bytes);
            writer
                .start_file(IPL_NAME, SimpleFileOptions::default())
                .expect("start IPL archive member");
            writer.write_all(b"invalid IPL").expect("write IPL member");
            writer.finish().expect("finish IPL archive");
        }
        bytes.set_position(0);
        let mut archive = ZipArchive::new(bytes).expect("read test archive");

        assert!(
            extract_verified_member(&mut archive, &directory, IPL_NAME, 8192, IPL_SHA256).is_err()
        );
        assert_eq!(
            fs::read(&destination).expect("existing asset preserved"),
            b"user file"
        );
        assert!(!destination.with_extension("verified-part").exists());
        fs::remove_dir_all(directory).expect("remove isolated asset directory");
    }

    #[test]
    fn missing_or_invalid_set_is_unavailable_with_setup_guidance() {
        let directory = std::env::temp_dir().join(format!(
            "spec-chum-next-assets-invalid-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock follows Unix epoch")
                .as_nanos()
        ));
        assert!(!NextAssets::available_in(&directory));
        let missing = NextAssets::discover_in(directory.clone()).expect_err("missing asset set");
        assert!(missing.to_string().contains("missing"));
        assert!(missing
            .to_string()
            .contains("Get official assets in ROM Setup"));

        fs::create_dir_all(&directory).expect("create isolated asset directory");
        fs::write(directory.join(SYSTEM_ARCHIVE), b"invalid archive")
            .expect("write invalid archive");
        assert!(!NextAssets::available_in(&directory));
        let invalid = NextAssets::discover_in(directory.clone()).expect_err("invalid asset set");
        assert!(invalid
            .to_string()
            .contains("does not match the pinned official asset"));
        assert!(invalid
            .to_string()
            .contains("Get official assets in ROM Setup"));
        fs::remove_dir_all(directory).expect("remove isolated asset directory");
    }

    #[test]
    fn invalid_existing_card_preserves_guest_data() {
        let directory = std::env::temp_dir().join(format!(
            "spec-chum-next-card-preserve-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock follows Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create isolated card directory");
        let card = directory.join(CARD_NAME);
        for size in [8, CARD_BYTES] {
            let mut file = File::create(&card).expect("create existing card");
            file.set_len(size).expect("set existing card size");
            file.write_all(b"SAVEDATA")
                .expect("write guest data marker");
            drop(file);
            fs::write(directory.join(CARD_MARKER), "wrong source")
                .expect("write bad source marker");
            let assets = NextAssets {
                directory: directory.clone(),
                ipl: directory.join(IPL_NAME),
                archive: directory.join(SYSTEM_ARCHIVE),
                card: card.clone(),
            };
            let error = assets
                .prepare_card()
                .expect_err("invalid card must not be replaced");
            assert!(error.to_string().contains("backing up guest data"));
            assert_eq!(fs::metadata(&card).expect("card retained").len(), size);
            let mut preserved = [0u8; 8];
            File::open(&card)
                .expect("card retained")
                .read_exact(&mut preserved)
                .expect("guest data retained");
            assert_eq!(&preserved, b"SAVEDATA");
        }
        fs::remove_dir_all(directory).expect("remove isolated card directory");
    }

    #[test]
    #[ignore = "requires the pinned, user-installed official Next distribution"]
    fn prepares_a_full_card_from_the_verified_distribution() {
        let assets = NextAssets::discover().expect("official assets installed");
        let card = assets
            .prepare_card()
            .expect("build FAT16 card from the archive");
        let mut file = File::open(card).expect("card exists");
        let mut mbr = [0u8; 512];
        file.read_exact(&mut mbr).expect("read MBR");
        assert_eq!(&mbr[510..], &[0x55, 0xaa]);
        assert_eq!(mbr[450], 0x06);
        assert_eq!(fs::metadata(card).expect("card metadata").len(), CARD_BYTES);
        let partition = Partition::new(
            File::open(card).expect("open card"),
            PARTITION_LBA as u64 * SECTOR_BYTES,
            CARD_BYTES - PARTITION_LBA as u64 * SECTOR_BYTES,
        );
        let fs = FileSystem::new(partition, FsOptions::new()).expect("read FAT16 card");
        let root = fs.root_dir();
        let mut archive = ZipArchive::new(File::open(&assets.archive).expect("open archive"))
            .expect("read archive");
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).expect("archive member");
            if entry.is_dir() {
                continue;
            }
            let mut on_card = root
                .open_file(entry.name())
                .expect("every archive file is on card");
            let mut expected_hash = Sha256::new();
            let mut actual_hash = Sha256::new();
            io::copy(&mut entry, &mut hash_writer(&mut expected_hash)).expect("hash archive file");
            io::copy(&mut on_card, &mut hash_writer(&mut actual_hash)).expect("hash card file");
            assert_eq!(
                actual_hash.finalize(),
                expected_hash.finalize(),
                "{}",
                entry.name()
            );
        }
    }
}
