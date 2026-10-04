//! Next FPGA IPL SD card and SPI protocol.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use thiserror::Error;

pub(super) const PORT_NEXT_SD_CS: u16 = 0x00e7;
pub(super) const PORT_NEXT_SD_DATA: u16 = 0x00eb;
pub(super) const SECTOR_SIZE: usize = 512;

#[derive(Clone, Copy, Debug)]
enum SdPhase {
    Idle,
    Argument {
        command: u8,
        received: u8,
        value: u32,
    },
    WaitResponse {
        command: u8,
        argument: u32,
        delay: u8,
    },
    StopArgument {
        received: u8,
    },
    Extra {
        bytes: [u8; 4],
        index: u8,
    },
    Status,
    ReadToken {
        lba: u32,
        delay: u8,
        multiple: bool,
    },
    RegisterToken {
        data: [u8; 18],
    },
    RegisterData {
        data: [u8; 18],
        index: u8,
    },
    StopResponse {
        delay: u8,
    },
    ReadData {
        index: u16,
        lba: u32,
        multiple: bool,
    },
    WriteToken {
        lba: u32,
        multiple: bool,
    },
    WriteData {
        index: u16,
        lba: u32,
        multiple: bool,
    },
    WriteResponse {
        value: u8,
        lba: u32,
        multiple: bool,
    },
    WriteBusy {
        cycles: u8,
        lba: u32,
        multiple: bool,
    },
}

/// Errors attaching or accessing a file-backed Next SD card.
#[derive(Debug, Error)]
pub enum NextSdError {
    #[error("Next SD image '{path}' must be a non-empty multiple of 512 bytes, got {length}")]
    InvalidLength {
        path: std::path::PathBuf,
        length: u64,
    },
    #[error("could not open Next SD image '{path}': {source}")]
    OpenImage {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not read metadata for Next SD image '{path}': {source}")]
    ImageMetadata {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Next SD sector {lba} is outside the {sectors}-sector image")]
    SectorOutOfRange { lba: u32, sectors: u64 },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug)]
pub(super) struct SdSpi {
    file: Arc<Mutex<File>>,
    len: u64,
    selected: bool,
    idle: bool,
    phase: SdPhase,
    app_command: bool,
    high_capacity: bool,
    last_exchange_t: Option<u64>,
    read_buffer: Box<[u8; SECTOR_SIZE]>,
    write_buffer: Box<[u8; SECTOR_SIZE]>,
}

impl SdSpi {
    pub(super) fn open(path: &Path) -> Result<Self, NextSdError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|source| NextSdError::OpenImage {
                path: path.to_path_buf(),
                source,
            })?;
        let len = file
            .metadata()
            .map_err(|source| NextSdError::ImageMetadata {
                path: path.to_path_buf(),
                source,
            })?
            .len();
        if len == 0 || !len.is_multiple_of(SECTOR_SIZE as u64) {
            return Err(NextSdError::InvalidLength {
                path: path.to_path_buf(),
                length: len,
            });
        }
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            len,
            selected: false,
            idle: true,
            phase: SdPhase::Idle,
            app_command: false,
            high_capacity: false,
            last_exchange_t: None,
            read_buffer: Box::new([0; SECTOR_SIZE]),
            write_buffer: Box::new([0; SECTOR_SIZE]),
        })
    }

    pub(super) fn select(&mut self, selected: bool) {
        if selected != self.selected {
            self.phase = SdPhase::Idle;
        }
        self.selected = selected;
    }

    pub(super) fn reset(&mut self) {
        self.selected = false;
        self.idle = true;
        self.phase = SdPhase::Idle;
        self.app_command = false;
        self.high_capacity = false;
        self.last_exchange_t = None;
    }

    pub(super) fn exchange_at(&mut self, byte: u8, t: u64) -> u8 {
        if !self.selected {
            return 0xff;
        }
        if let Some(last_t) = self.last_exchange_t {
            let elapsed = t.saturating_sub(last_t);
            self.phase = match self.phase {
                SdPhase::ReadToken {
                    lba,
                    delay,
                    multiple,
                } => SdPhase::ReadToken {
                    lba,
                    delay: delay.saturating_sub(elapsed.min(u64::from(u8::MAX)) as u8),
                    multiple,
                },
                phase => phase,
            };
        }
        self.last_exchange_t = Some(t);
        if byte == 0x4c && matches!(self.phase, SdPhase::ReadToken { multiple: true, .. }) {
            self.phase = SdPhase::StopArgument { received: 0 };
            return 0xff;
        }
        match self.phase {
            SdPhase::Idle => {
                if byte & 0xc0 == 0x40 {
                    self.phase = SdPhase::Argument {
                        command: byte & 0x3f,
                        received: 0,
                        value: 0,
                    };
                }
                0xff
            }
            SdPhase::StopResponse { delay } => {
                if delay > 0 {
                    self.phase = SdPhase::StopResponse { delay: delay - 1 };
                    0xff
                } else {
                    self.phase = SdPhase::Idle;
                    0
                }
            }
            SdPhase::Argument {
                command,
                received,
                value,
            } => {
                self.phase = if received < 4 {
                    SdPhase::Argument {
                        command,
                        received: received + 1,
                        value: (value << 8) | u32::from(byte),
                    }
                } else {
                    SdPhase::WaitResponse {
                        command,
                        argument: value,
                        delay: 1,
                    }
                };
                0xff
            }
            SdPhase::StopArgument { received } => {
                self.phase = if received < 4 {
                    SdPhase::StopArgument {
                        received: received + 1,
                    }
                } else {
                    // FatFs clocks eight bytes after CMD12 before polling R1.
                    SdPhase::StopResponse { delay: 8 }
                };
                0xff
            }
            SdPhase::WaitResponse {
                command,
                argument,
                delay,
            } => {
                if delay > 0 {
                    self.phase = SdPhase::WaitResponse {
                        command,
                        argument,
                        delay: delay - 1,
                    };
                    0xff
                } else {
                    self.respond(command, argument)
                }
            }
            SdPhase::Extra { bytes, index } => {
                let value = bytes[usize::from(index)];
                self.phase = if index == 3 {
                    SdPhase::Idle
                } else {
                    SdPhase::Extra {
                        bytes,
                        index: index + 1,
                    }
                };
                value
            }
            SdPhase::Status => {
                self.phase = SdPhase::Idle;
                0
            }
            SdPhase::ReadToken {
                lba,
                delay,
                multiple,
            } => {
                if delay > 0 {
                    self.phase = SdPhase::ReadToken {
                        lba,
                        delay: delay - 1,
                        multiple,
                    };
                    0xff
                } else {
                    let mut data = Box::new([0; SECTOR_SIZE]);
                    if self.read_sector(lba, &mut data).is_err() {
                        self.phase = SdPhase::Idle;
                        0x04
                    } else {
                        self.read_buffer = data;
                        self.phase = SdPhase::ReadData {
                            index: 0,
                            lba,
                            multiple,
                        };
                        0xfe
                    }
                }
            }
            SdPhase::RegisterToken { data } => {
                self.phase = SdPhase::RegisterData { data, index: 0 };
                0xfe
            }
            SdPhase::RegisterData { data, index } => {
                let value = data[usize::from(index)];
                self.phase = if index == 17 {
                    SdPhase::Idle
                } else {
                    SdPhase::RegisterData {
                        data,
                        index: index + 1,
                    }
                };
                value
            }
            SdPhase::ReadData {
                index,
                lba,
                multiple,
            } => {
                if index < 512 {
                    let value = self.read_buffer[usize::from(index)];
                    self.phase = SdPhase::ReadData {
                        index: index + 1,
                        lba,
                        multiple,
                    };
                    value
                } else if index < 514 {
                    self.phase = if index == 513 {
                        if multiple {
                            SdPhase::ReadToken {
                                lba: lba + 1,
                                delay: 0,
                                multiple,
                            }
                        } else {
                            SdPhase::Idle
                        }
                    } else {
                        SdPhase::ReadData {
                            index: index + 1,
                            lba,
                            multiple,
                        }
                    };
                    0xff
                } else {
                    0xff
                }
            }
            SdPhase::WriteToken { lba, multiple } => {
                if multiple && byte == 0xfd {
                    self.phase = SdPhase::StopResponse { delay: 0 };
                } else if byte == if multiple { 0xfc } else { 0xfe } {
                    self.write_buffer.fill(0);
                    self.phase = SdPhase::WriteData {
                        index: 0,
                        lba,
                        multiple,
                    };
                }
                0xff
            }
            SdPhase::WriteData {
                index,
                lba,
                multiple,
            } => {
                match index {
                    0..512 => {
                        self.write_buffer[usize::from(index)] = byte;
                        self.phase = SdPhase::WriteData {
                            index: index + 1,
                            lba,
                            multiple,
                        };
                    }
                    512 => {
                        self.phase = SdPhase::WriteData {
                            index: index + 1,
                            lba,
                            multiple,
                        };
                    }
                    _ => {
                        let response = if self.write_sector(lba).is_ok() {
                            0x05
                        } else {
                            0x0d
                        };
                        self.phase = SdPhase::WriteResponse {
                            value: response,
                            lba,
                            multiple,
                        };
                    }
                }
                0xff
            }
            SdPhase::WriteResponse {
                value,
                lba,
                multiple,
            } => {
                self.phase = SdPhase::WriteBusy {
                    cycles: 1,
                    lba,
                    multiple,
                };
                value
            }
            SdPhase::WriteBusy {
                cycles,
                lba,
                multiple,
            } => {
                if cycles > 0 {
                    self.phase = SdPhase::WriteBusy {
                        cycles: cycles - 1,
                        lba,
                        multiple,
                    };
                    0x00
                } else {
                    self.phase = if multiple {
                        SdPhase::WriteToken {
                            lba: lba.saturating_add(1),
                            multiple,
                        }
                    } else {
                        SdPhase::Idle
                    };
                    0xff
                }
            }
        }
    }

    fn respond(&mut self, command: u8, argument: u32) -> u8 {
        let app = std::mem::take(&mut self.app_command);
        let (response, next) = if app && command == 41 {
            self.idle = false;
            self.high_capacity = argument & 0x4000_0000 != 0;
            (0, SdPhase::Idle)
        } else {
            match command {
                0 => {
                    self.idle = true;
                    self.high_capacity = false;
                    (1, SdPhase::Idle)
                }
                12 if !self.idle => (0, SdPhase::StopResponse { delay: 1 }),
                13 if !self.idle => (0, SdPhase::Status),
                8 => (
                    u8::from(self.idle),
                    SdPhase::Extra {
                        bytes: [0, 0, ((argument >> 8) as u8) & 0x0f, argument as u8],
                        index: 0,
                    },
                ),
                9 if !self.idle => (0, SdPhase::RegisterToken { data: self.csd() }),
                16 => (u8::from(self.idle), SdPhase::Idle),
                17 | 18 | 24 | 25 if !self.idle => {
                    let lba = self.argument_to_lba(argument);
                    if self.lba_is_valid(lba) {
                        let next = match command {
                            17 | 18 => SdPhase::ReadToken {
                                lba,
                                delay: 1,
                                multiple: command == 18,
                            },
                            _ => SdPhase::WriteToken {
                                lba,
                                multiple: command == 25,
                            },
                        };
                        (0, next)
                    } else {
                        (4, SdPhase::Idle)
                    }
                }
                55 => {
                    self.app_command = true;
                    (u8::from(self.idle), SdPhase::Idle)
                }
                58 => (
                    u8::from(self.idle),
                    SdPhase::Extra {
                        bytes: [
                            if self.idle {
                                0
                            } else if self.high_capacity {
                                0xc0
                            } else {
                                0x80
                            },
                            0xff,
                            0x80,
                            0,
                        ],
                        index: 0,
                    },
                ),
                12 if !self.idle => (0, SdPhase::Idle),
                _ => (if self.idle { 5 } else { 4 }, SdPhase::Idle),
            }
        };
        self.phase = next;
        response
    }

    fn csd(&self) -> [u8; 18] {
        let sectors = self.len / SECTOR_SIZE as u64;
        let c_size = sectors.div_ceil(1024).saturating_sub(1).min(0x3f_ffff);
        let register = (1u128 << 126)
            | (0x0eu128 << 112)
            | (0x32u128 << 96)
            | (0x5b5u128 << 84)
            | (9u128 << 80)
            | (u128::from(c_size) << 48)
            | (1u128 << 46)
            | (0x7fu128 << 39)
            | (2u128 << 26)
            | (9u128 << 22)
            | 1;
        let mut data = [0xff; 18];
        data[..16].copy_from_slice(&register.to_be_bytes());
        let mut crc = 0u8;
        for &byte in &data[..15] {
            let mut value = byte;
            for _ in 0..8 {
                let feedback = ((crc >> 6) ^ (value >> 7)) & 1;
                crc = (crc << 1) & 0x7f;
                if feedback != 0 {
                    crc ^= 0x09;
                }
                value <<= 1;
            }
        }
        data[15] = (crc << 1) | 1;
        let mut data_crc = 0u16;
        for &byte in &data[..16] {
            data_crc ^= u16::from(byte) << 8;
            for _ in 0..8 {
                data_crc = if data_crc & 0x8000 == 0 {
                    data_crc << 1
                } else {
                    (data_crc << 1) ^ 0x1021
                };
            }
        }
        data[16] = (data_crc >> 8) as u8;
        data[17] = data_crc as u8;
        data
    }

    pub(super) fn read_sector(
        &mut self,
        lba: u32,
        output: &mut [u8; SECTOR_SIZE],
    ) -> Result<(), NextSdError> {
        let end = self.sector_end(lba)?;
        let mut file = self
            .file
            .lock()
            .map_err(|_| std::io::Error::other("Next SD file lock poisoned"))?;
        file.seek(SeekFrom::Start(end - SECTOR_SIZE as u64))?;
        file.read_exact(output)?;
        Ok(())
    }

    fn write_sector(&mut self, lba: u32) -> Result<(), NextSdError> {
        let end = self.sector_end(lba)?;
        let mut file = self
            .file
            .lock()
            .map_err(|_| std::io::Error::other("Next SD file lock poisoned"))?;
        file.seek(SeekFrom::Start(end - SECTOR_SIZE as u64))?;
        file.write_all(&self.write_buffer[..])?;
        file.flush()?;
        Ok(())
    }

    fn sector_end(&self, lba: u32) -> Result<u64, NextSdError> {
        let end = u64::from(lba)
            .checked_mul(SECTOR_SIZE as u64)
            .and_then(|offset| offset.checked_add(SECTOR_SIZE as u64))
            .ok_or(NextSdError::SectorOutOfRange {
                lba,
                sectors: self.len / SECTOR_SIZE as u64,
            })?;
        if end > self.len {
            return Err(NextSdError::SectorOutOfRange {
                lba,
                sectors: self.len / SECTOR_SIZE as u64,
            });
        }
        Ok(end)
    }

    fn argument_to_lba(&self, argument: u32) -> u32 {
        if self.high_capacity {
            argument
        } else {
            argument / SECTOR_SIZE as u32
        }
    }

    fn lba_is_valid(&self, lba: u32) -> bool {
        u64::from(lba) * SECTOR_SIZE as u64 + SECTOR_SIZE as u64 <= self.len
    }
}
