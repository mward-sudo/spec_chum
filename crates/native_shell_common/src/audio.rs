//! Shared PCM ring and CPAL output stream for native shells.

use std::collections::VecDeque;
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use parking_lot::Mutex;

pub type OutputStream = cpal::Stream;

#[derive(Clone, Copy, Debug)]
pub enum ShellPlatform {
    Linux,
    Windows,
}

impl ShellPlatform {
    fn name(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Windows => "windows",
        }
    }
}

/// Shared ring fed each emulated frame and drained by the audio callback.
#[derive(Debug)]
pub struct PcmRing {
    samples: VecDeque<[f32; 2]>,
    output_rate: u32,
    sample_phase: u64,
    pub volume: f32,
    pub muted: bool,
}

impl Default for PcmRing {
    fn default() -> Self {
        Self::new()
    }
}

impl PcmRing {
    #[must_use]
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(8192),
            output_rate: 44_100,
            sample_phase: 0,
            volume: 0.7,
            muted: false,
        }
    }

    pub fn push_frame(&mut self, pcm: &[f32]) {
        self.trim_for(pcm.len());
        self.samples.extend(
            pcm.iter()
                .skip(pcm.len().saturating_sub(44_100))
                .map(|&sample| [sample, sample]),
        );
    }

    pub fn push_stereo_frame(&mut self, pcm: &[f32]) {
        let frame_count = pcm.len() / 2;
        self.trim_for(frame_count);
        self.samples.extend(
            pcm.as_chunks::<2>()
                .0
                .iter()
                .skip(frame_count.saturating_sub(44_100))
                .copied(),
        );
    }

    fn trim_for(&mut self, frame_count: usize) {
        // Soft cap so a stalled consumer cannot grow unbounded.
        const MAX: usize = 44_100; // ~1s stereo frames
        if frame_count >= MAX {
            self.samples.clear();
        } else if self.samples.len() + frame_count > MAX {
            let drop_n = self.samples.len() + frame_count - MAX;
            self.samples.drain(0..drop_n);
        }
    }

    fn set_output_rate(&mut self, output_rate: u32) {
        self.output_rate = output_rate.max(1);
        self.sample_phase = 0;
    }

    fn pop_frame(&mut self) -> [f32; 2] {
        let Some(&first) = self.samples.front() else {
            return [0.0; 2];
        };
        let second = self.samples.get(1).copied().unwrap_or(first);
        let fraction = self.sample_phase as f32 / self.output_rate as f32;
        let pair = [
            first[0] + (second[0] - first[0]) * fraction,
            first[1] + (second[1] - first[1]) * fraction,
        ];
        self.sample_phase += 44_100;
        let consumed = (self.sample_phase / u64::from(self.output_rate)) as usize;
        self.sample_phase %= u64::from(self.output_rate);
        for _ in 0..consumed {
            self.samples.pop_front();
        }
        if self.muted {
            return [0.0; 2];
        }
        let gain = self.volume.clamp(0.0, 1.0);
        [
            (pair[0] * gain).clamp(-1.0, 1.0),
            (pair[1] * gain).clamp(-1.0, 1.0),
        ]
    }
}

/// Start a CPAL output stream, supporting the common signed, unsigned, and float formats.
pub fn start_stream(ring: Arc<Mutex<PcmRing>>, platform: ShellPlatform) -> Option<OutputStream> {
    let platform = platform.name();
    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(device) => device,
        None => {
            eprintln!("spec-chum-{platform}: no default audio output device");
            return None;
        }
    };
    let config = match device.default_output_config() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("spec-chum-{platform}: audio config failed: {error}");
            return None;
        }
    };
    let channel_count = config.channels();
    ring.lock().set_output_rate(config.sample_rate().0);
    let stream_config = config.config();
    let stream = match config.sample_format() {
        SampleFormat::F32 => {
            build_stream::<f32>(&device, &stream_config, channel_count, ring, platform)
        }
        SampleFormat::I16 => {
            build_stream::<i16>(&device, &stream_config, channel_count, ring, platform)
        }
        SampleFormat::U16 => {
            build_stream::<u16>(&device, &stream_config, channel_count, ring, platform)
        }
        other => {
            eprintln!("spec-chum-{platform}: unsupported audio sample format {other:?}");
            return None;
        }
    }?;
    if let Err(error) = stream.play() {
        eprintln!("spec-chum-{platform}: audio play failed: {error}");
        return None;
    }
    Some(stream)
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channel_count: u16,
    ring: Arc<Mutex<PcmRing>>,
    platform: &'static str,
) -> Option<cpal::Stream>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    match device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let mut samples = ring.lock();
            fill_output_samples(data, usize::from(channel_count), &mut samples);
        },
        move |error| eprintln!("spec-chum-{platform} audio error: {error}"),
        None,
    ) {
        Ok(stream) => Some(stream),
        Err(error) => {
            eprintln!("spec-chum-{platform}: build audio stream failed: {error}");
            None
        }
    }
}

fn fill_output_samples<T>(data: &mut [T], channels: usize, ring: &mut PcmRing)
where
    T: SizedSample + FromSample<f32>,
{
    for frame in data.chunks_mut(channels.max(1)) {
        let pair = ring.pop_frame();
        let mono = f32::midpoint(pair[0], pair[1]);
        let mono_output = frame.len() == 1;
        for (channel, output) in frame.iter_mut().enumerate() {
            let sample = if mono_output {
                mono
            } else {
                pair[channel.min(1)]
            };
            *output = T::from_sample(sample);
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

    #[test]
    fn preserves_stereo_channels_and_downmixes_mono_output() {
        let mut ring = PcmRing::new();
        ring.volume = 1.0;
        ring.push_stereo_frame(&[0.25, -0.75, 0.5, 0.0]);

        let mut stereo = [0.0_f32; 4];
        fill_output_samples(&mut stereo, 2, &mut ring);
        assert_eq!(stereo, [0.25, -0.75, 0.5, 0.0]);

        ring.push_stereo_frame(&[0.25, -0.75]);
        let mut mono = [0.0_f32; 1];
        fill_output_samples(&mut mono, 1, &mut ring);
        assert_eq!(mono, [-0.25]);
    }

    #[test]
    fn resamples_source_frames_to_device_rate_and_preserves_stereo() {
        let mut ring = PcmRing::new();
        ring.volume = 1.0;
        ring.set_output_rate(48_000);
        ring.push_stereo_frame(&[0.0, 0.5, 1.0, -0.5, 0.0, 0.5]);

        let mut output = [0.0_f32; 8];
        fill_output_samples(&mut output, 2, &mut ring);
        assert_eq!(output[0..2], [0.0, 0.5]);
        assert!((output[2] - 0.91875).abs() < 1e-6);
        assert!((output[3] + 0.41875).abs() < 1e-6);
        assert!((output[4] - 0.1625).abs() < 1e-6);
        assert!((output[5] - 0.3375).abs() < 1e-6);
        assert_eq!(output[6..8], [0.0, 0.5]);
    }

    #[test]
    fn resampling_keeps_source_and_device_clocks_in_step() {
        let mut ring = PcmRing::new();
        ring.set_output_rate(48_000);
        ring.samples
            .extend((0..44_102).map(|index| [index as f32, 0.0]));

        for _ in 0..48_000 {
            ring.pop_frame();
        }

        assert_eq!(ring.samples.len(), 2);
    }
}
