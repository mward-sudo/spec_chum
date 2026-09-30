use super::*;
use std::path::PathBuf;
use tape::{tap_checksum, TapImage};
use ula::{FRAME_TSTATES_48, INT_LENGTH_48};
use z80::{Io, Memory};

#[path = "tests/fixtures.rs"]
mod fixtures;
use fixtures::*;

#[path = "tests/debugger.rs"]
mod debugger;
#[path = "tests/disk.rs"]
mod disk;
#[path = "tests/interface1.rs"]
mod interface1;
#[path = "tests/models.rs"]
mod models;
#[path = "tests/peripherals.rs"]
mod peripherals;
#[path = "tests/recordings.rs"]
mod recordings;
#[path = "tests/snapshots.rs"]
mod snapshots;
#[path = "tests/tape_compat.rs"]
mod tape_compat;
#[path = "tests/tape_deck.rs"]
mod tape_deck;
#[path = "tests/tape_loading.rs"]
mod tape_loading;
use recordings::minimal_tzx_turbo_machine;
use tape_loading::attr_mark_code_ok;
#[path = "tests/timing_video.rs"]
mod timing_video;
#[path = "tests/trdos.rs"]
mod trdos;
