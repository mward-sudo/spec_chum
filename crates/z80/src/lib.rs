//! Spec Chum Z80 CPU — from-scratch cycle-aware implementation.

#![allow(clippy::pedantic)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]

mod bus;
mod cpu;
mod flags;
mod opcodes;
mod registers;

pub use bus::{FlatMem, Io, Memory, NullIo};
pub use cpu::Cpu;
pub use registers::{flag, Registers};

#[cfg(test)]
mod fuse;
