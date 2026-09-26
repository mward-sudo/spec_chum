//! Host PCM playback from [`HostSession::audio_pcm`] snapshots.

use std::collections::VecDeque;
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
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
    let config = match device.default_output_config() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("spec-chum-windows: audio config failed: {e}");
            return None;
        }
    };
    let channels = config.channels();
    let stream_config = config.config();
    let stream = match config.sample_format() {
        SampleFormat::F32 => build_stream::<f32>(&device, &stream_config, channels, ring),
        SampleFormat::I16 => build_stream::<i16>(&device, &stream_config, channels, ring),
        SampleFormat::U16 => build_stream::<u16>(&device, &stream_config, channels, ring),
        other => {
            eprintln!("spec-chum-windows: unsupported audio sample format {other:?}");
            return None;
        }
    }?;
    if let Err(e) = stream.play() {
        eprintln!("spec-chum-windows: audio play failed: {e}");
        return None;
    }
    Some(stream)
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: u16,
    ring: Arc<Mutex<PcmRing>>,
) -> Option<cpal::Stream>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    match device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let mut st = ring.lock();
            fill_output_samples(data, usize::from(channels), &mut st);
        },
        |err| eprintln!("spec-chum-windows audio error: {err}"),
        None,
    ) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("spec-chum-windows: build audio stream failed: {e}");
            None
        }
    }
}

fn fill_output_samples<T>(data: &mut [T], channels: usize, ring: &mut PcmRing)
where
    T: SizedSample + FromSample<f32>,
{
    for frame in data.chunks_mut(channels.max(1)) {
        let sample = T::from_sample(ring.pop_sample());
        frame[0] = sample;
        for out in frame.iter_mut().skip(1) {
            *out = sample;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fill_output_samples, PcmRing};

    #[test]
    fn fills_f32_i16_and_u16_output_without_audio_device() {
        let mut ring = PcmRing::new();
        ring.volume = 1.0;
        ring.push_frame(&[0.5, -0.5]);

        let mut f32_output = [0.0_f32; 4];
        fill_output_samples(&mut f32_output, 2, &mut ring);
        assert_eq!(f32_output, [0.5, 0.5, -0.5, -0.5]);

        ring.push_frame(&[0.5, -0.5]);
        let mut i16_output = [0_i16; 2];
        fill_output_samples(&mut i16_output, 1, &mut ring);
        assert_eq!(i16_output, [16_384, -16_384]);

        ring.push_frame(&[0.5, -0.5]);
        let mut u16_output = [0_u16; 2];
        fill_output_samples(&mut u16_output, 1, &mut ring);
        assert_eq!(u16_output, [49_152, 16_384]);
    }
}
