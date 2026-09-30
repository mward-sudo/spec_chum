//! Tape playback state and loading options shared by machine models.

use tape::{TapPlayer, TzxPlayer};

/// Inserted tape image (TAP pulse player or TZX pulse player).
#[derive(Clone, Debug)]
pub enum TapeDeck {
    Tap(TapPlayer),
    Tzx(TzxPlayer),
}

impl TapeDeck {
    pub fn advance(&mut self, dt: u32) -> bool {
        match self {
            Self::Tap(t) => t.advance(dt),
            Self::Tzx(t) => t.advance(dt),
        }
    }

    #[must_use]
    pub fn block(&self) -> Option<usize> {
        match self {
            Self::Tap(t) => Some(t.block),
            Self::Tzx(t) => Some(t.block),
        }
    }

    #[must_use]
    pub fn block_count(&self) -> usize {
        match self {
            Self::Tap(t) => t.image.blocks.len(),
            Self::Tzx(t) => t.block_count(),
        }
    }

    #[must_use]
    pub fn pulse_index(&self) -> usize {
        match self {
            Self::Tap(t) => t.pulse_index(),
            // TZX: progress is within the active block, not the whole schedule.
            Self::Tzx(t) => t.active_pulse_index(),
        }
    }

    #[must_use]
    pub fn pulse_count(&self) -> usize {
        match self {
            Self::Tap(t) => t.scheduled_pulses(),
            Self::Tzx(t) => t.active_pulse_count(),
        }
    }

    pub fn as_tap_mut(&mut self) -> Option<&mut TapPlayer> {
        match self {
            Self::Tap(t) => Some(t),
            Self::Tzx(_) => None,
        }
    }

    #[must_use]
    pub fn as_tap(&self) -> Option<&TapPlayer> {
        match self {
            Self::Tap(t) => Some(t),
            Self::Tzx(_) => None,
        }
    }

    /// True when the deck exposes TAP blocks an LD-BYTES trap can poke.
    ///
    /// Pulse-only TZX decks (custom loaders such as Speedlock) have no block
    /// list, so Instant cannot flash them — they must load off the EAR
    /// bitstream instead.
    #[must_use]
    pub fn supports_flash_load(&self) -> bool {
        matches!(self, Self::Tap(_))
    }

    pub fn set_playing(&mut self, playing: bool) {
        match self {
            Self::Tap(t) => t.set_playing(playing),
            Self::Tzx(t) => t.set_playing(playing),
        }
    }

    #[must_use]
    pub fn playing(&self) -> bool {
        match self {
            Self::Tap(t) => t.playing,
            Self::Tzx(t) => t.playing,
        }
    }

    /// True when the deck has no remaining bitstream / TAP blocks to play.
    #[must_use]
    pub fn finished(&self) -> bool {
        match self {
            Self::Tap(t) => t.finished(),
            Self::Tzx(t) => t.finished(),
        }
    }

    pub fn rewind(&mut self) {
        match self {
            Self::Tap(t) => t.rewind(),
            Self::Tzx(t) => t.rewind(),
        }
    }
}

/// EAR rate used when Instant is asked for but the deck cannot flash-load.
///
/// Pulse-only TZX decks have no LD-BYTES trap to poke, so Instant falls back to
/// the EAR bitstream. It stays *instant-ish* by running the maximum turbo
/// instead of inheriting the EAR speed control the user picked for Play (#390).
pub const INSTANT_EAR_FALLBACK_SPEED: u32 = 64;

/// User controls for tape loading speed / instant flash-load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TapeLoadOptions {
    /// When true, TAP decks trap at LD-BYTES and poke bytes immediately.
    ///
    /// Decks without TAP blocks (pulse TZX) cannot be flashed; they load off
    /// the EAR bitstream at [`INSTANT_EAR_FALLBACK_SPEED`] instead of
    /// [`Self::speed`].
    pub flash_load: bool,
    /// EAR bitstream speed multiplier (`1` = realtime). Clamped to `1..=64`.
    ///
    /// While a tape is **playing** on the EAR path (flash-load off), each
    /// [`crate::Machine::run_frame`] executes this many Spectrum frames so wall-clock
    /// load time ≈ realtime / speed. Pulse widths stay ROM-accurate (CPU↔tape
    /// 1:1); Instant on a flashable deck is unchanged (single frame per call).
    pub speed: u32,
    /// ~20s-class load: hybrid flash + cosmetic abbreviated pilots/border on
    /// flashable TAP decks ([#167](https://github.com/mward-sudo/spec_chum/issues/167));
    /// EAR-fallback decks keep abbreviated pauses at [`tape::EXPERIENCE_EAR_SPEED`]
    /// ([#82](https://github.com/mward-sudo/spec_chum/issues/82)). Mutually exclusive
    /// with sticky Instant [`Self::flash_load`] in the UI (Experience still uses the
    /// LD-BYTES flash trap internally).
    pub experience_load: bool,
}

impl Default for TapeLoadOptions {
    fn default() -> Self {
        // EAR path by default; UI Instant actions enable flash-load ephemerally.
        Self {
            flash_load: false,
            speed: 1,
            experience_load: false,
        }
    }
}

impl TapeLoadOptions {
    #[must_use]
    pub fn with_speed(mut self, speed: u32) -> Self {
        self.speed = speed.clamp(1, 64);
        self.experience_load = false;
        self
    }

    #[must_use]
    pub fn experience() -> Self {
        Self {
            flash_load: false,
            speed: tape::EXPERIENCE_EAR_SPEED,
            experience_load: true,
        }
    }

    pub(super) fn normalized(mut self) -> Self {
        if self.experience_load {
            self.flash_load = false;
            self.speed = tape::EXPERIENCE_EAR_SPEED;
        } else if self.flash_load {
            self.experience_load = false;
        }
        self.speed = self.speed.clamp(1, 64);
        self
    }

    /// Instant or hybrid Experience: LD-BYTES trap pokes TAP bytes.
    #[must_use]
    pub fn ld_bytes_flash_trap(self) -> bool {
        self.flash_load || self.experience_load
    }

    /// Skip advancing TAP EAR while Instant flash or hybrid Experience owns the
    /// block stream.
    ///
    /// Experience keeps this true for the whole mode (not only while
    /// `experience_pending` is set): advancing EAR between LD-BYTES traps would
    /// consume later TAP blocks before the next flash (same Boggit/`0xC8` hazard
    /// Instant guards against). Pulse-only decks are not TAP, so they still
    /// advance under Experience EAR-fallback.
    #[must_use]
    pub fn skip_tap_ear_advance(self) -> bool {
        self.flash_load || self.experience_load
    }
}

/// Tape position for UI progress (block + pulse within the current schedule).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TapeProgress {
    pub block_index: u32,
    pub block_count: u32,
    pub pulse_index: u32,
    pub pulse_count: u32,
}

impl TapeProgress {
    /// 0.0..1.0 overall position estimate (block + intra-block pulse fraction).
    #[must_use]
    pub fn fraction(&self) -> f32 {
        if self.block_count == 0 {
            return 0.0;
        }
        let block = self.block_index.min(self.block_count) as f32;
        let within = if self.pulse_count == 0 {
            // Consumed trailing zero-pulse block (e.g. 0x20 pause_ms=0): treat as done.
            if self.block_index.saturating_add(1) >= self.block_count {
                1.0
            } else {
                0.0
            }
        } else {
            self.pulse_index.min(self.pulse_count) as f32 / self.pulse_count as f32
        };
        ((block + within) / self.block_count as f32).clamp(0.0, 1.0)
    }
}
