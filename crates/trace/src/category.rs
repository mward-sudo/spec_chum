//! Trace category bitflags and list parsing.

/// Trace categories (bitflags). Combine with `|`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Category(u64);

impl Category {
    pub const NONE: Self = Self(0);
    pub const CPU: Self = Self(1 << 0);
    pub const BUS: Self = Self(1 << 1);
    pub const TAPE: Self = Self(1 << 2);
    pub const ULA: Self = Self(1 << 3);
    pub const MACHINE: Self = Self(1 << 4);
    pub const AY: Self = Self(1 << 5);
    pub const DISK: Self = Self(1 << 6);
    pub const MEM: Self = Self(1 << 7);
    /// Convenience: BUS|TAPE|ULA|MACHINE (excludes high-volume CPU, AY, DISK, MEM).
    pub const DEFAULT: Self = Self(Self::BUS.0 | Self::TAPE.0 | Self::ULA.0 | Self::MACHINE.0);
    pub const ALL: Self = Self(
        Self::CPU.0
            | Self::BUS.0
            | Self::TAPE.0
            | Self::ULA.0
            | Self::MACHINE.0
            | Self::AY.0
            | Self::DISK.0
            | Self::MEM.0,
    );

    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    #[must_use]
    pub(crate) const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub fn parse_list(s: &str) -> Self {
        let mut c = Self::NONE;
        for part in s.split(|ch: char| ch == ',' || ch.is_whitespace()) {
            let p = part.trim().to_ascii_lowercase();
            if p.is_empty() {
                continue;
            }
            c = c.union(match p.as_str() {
                "all" => Self::ALL,
                "default" | "debug" => Self::DEFAULT,
                "cpu" | "z80" => Self::CPU,
                "bus" | "io" => Self::BUS,
                "tape" => Self::TAPE,
                "ula" | "video" => Self::ULA,
                "machine" | "mach" => Self::MACHINE,
                "ay" | "psg" => Self::AY,
                "disk" | "fdc" => Self::DISK,
                "mem" | "memory" => Self::MEM,
                _ => Self::NONE,
            });
        }
        c
    }
}

impl std::ops::BitOr for Category {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for Category {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}
