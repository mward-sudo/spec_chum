//! Host PCM playback from [`HostSession::audio_pcm`] snapshots.

use std::collections::VecDeque;
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;

/// Shared ring fed each emulated frame; drained by the cpal callback.
#[derive(Debug, Default)]
pub struct PcmRing {
    samples: VecDeque<f32>,
    pub volume: f32,
    pub muted: bool,
}

impl PcmRing {
    #[must_use]
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(8192),
            volume: 0.7,
            muted: false,
        }
    }

    pub fn push_frame(&mut self, pcm: &[f32]) {
        // Soft cap so a stalled consumer cannot grow unbounded.
        const MAX: usize = 44_100; // ~1s mono
        if self.samples.len() > MAX {
            let drop_n = self.samples.len() - MAX / 2;
            self.samples.drain(0..drop_n);
        }
        self.samples.extend(pcm.iter().copied());
    }

    fn pop_sample(&mut self) -> f32 {
        let s = self.samples.pop_front().unwrap_or(0.0);
        if self.muted {
            return 0.0;
        }
        (s * self.volume.clamp(0.0, 1.0)).clamp(-1.0, 1.0)
    }
}

/// Start a cpal output stream that drains `ring`. Returns `None` when no device.
pub fn start_stream(ring: Arc<Mutex<PcmRing>>) -> Option<cpal::Stream> {
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let config = device.default_output_config().ok()?;
    let channels = config.channels();
    let stream = device
        .build_output_stream(
            &config.config(),
            move |data: &mut [f32], _| {
                let mut st = ring.lock();
                let ch = usize::from(channels.max(1));
                for frame in data.chunks_mut(ch) {
                    let s = st.pop_sample();
                    frame[0] = s;
                    for out in frame.iter_mut().skip(1) {
                        *out = s;
                    }
                }
            },
            |err| eprintln!("spec-chum-windows audio error: {err}"),
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}
