//! Memory and I/O traits with T-state accounting hooks.

/// Host memory. Contended wait states are applied by the bus implementation
/// via the returned extra T-states (usually 0 for Fuse flat memory).
pub trait Memory {
    fn read(&mut self, addr: u16, t: u64) -> (u8, u32);
    fn write(&mut self, addr: u16, value: u8, t: u64) -> u32;

    /// Opcode fetch hook. Buses with instruction-fetch-triggered hardware may
    /// distinguish M1 cycles from ordinary memory reads.
    fn read_opcode(&mut self, addr: u16, t: u64) -> (u8, u32) {
        self.read(addr, t)
    }

    /// M1 refresh at T4 of opcode fetch (48K ULA snow hook).
    fn m1_refresh(&mut self, _refresh_addr: u16, _t: u64, _m1_contended: bool) {}
}

/// Host I/O ports.
pub trait Io {
    fn in_port(&mut self, port: u16, t: u64) -> (u8, u32);
    fn out_port(&mut self, port: u16, value: u8, t: u64) -> u32;

    /// Z80N NEXTREG write. Hosts with Next registers override this hook;
    /// classic Z80 bus implementations need no extra behavior.
    fn nextreg_write(&mut self, _register: u8, _value: u8, _t: u64) {}
}

/// Flat 64K RAM used by unit / Fuse tests.
#[derive(Clone, Debug)]
pub struct FlatMem {
    pub data: Box<[u8; 65536]>,
}

impl Default for FlatMem {
    fn default() -> Self {
        Self {
            data: Box::new([0; 65536]),
        }
    }
}

impl FlatMem {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Memory for FlatMem {
    #[inline]
    fn read(&mut self, addr: u16, _t: u64) -> (u8, u32) {
        (self.data[addr as usize], 0)
    }

    #[inline]
    fn write(&mut self, addr: u16, value: u8, _t: u64) -> u32 {
        self.data[addr as usize] = value;
        0
    }
}

impl Io for FlatMem {
    fn in_port(&mut self, _port: u16, _t: u64) -> (u8, u32) {
        (0xff, 0)
    }

    fn out_port(&mut self, _port: u16, _value: u8, _t: u64) -> u32 {
        0
    }
}

/// I/O that always returns `0xFF` and ignores writes (Fuse default).
#[derive(Clone, Copy, Debug, Default)]
pub struct NullIo;

impl Io for NullIo {
    fn in_port(&mut self, _port: u16, _t: u64) -> (u8, u32) {
        (0xff, 0)
    }

    fn out_port(&mut self, _port: u16, _value: u8, _t: u64) -> u32 {
        0
    }
}
