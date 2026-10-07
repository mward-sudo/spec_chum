//! Host beeper and AY audio output state.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use machine::FrameAudio;

const NEXT_AUDIO_SAMPLE_RATE: f32 = 44_100.0;

pub(super) struct BeeperState {
    classic_samples: VecDeque<(f32, f32)>,
    next_samples: VecDeque<(f32, f32)>,
    next_sample_phase: f32,
    next_timing: bool,
    level: bool,
    sample_rate: u32,
    channels: u16,
    muted: bool,
    /// Linear host output gain 0…1.
    volume: f32,
}

impl Default for BeeperState {
    fn default() -> Self {
        Self {
            classic_samples: VecDeque::new(),
            next_samples: VecDeque::new(),
            next_sample_phase: 0.0,
            next_timing: false,
            level: false,
            sample_rate: 44100,
            channels: 2,
            muted: false,
            volume: 1.0,
        }
    }
}

impl BeeperState {
    pub(super) fn with_preferences(muted: bool, volume: f32) -> Self {
        Self {
            muted,
            volume,
            ..Self::default()
        }
    }

    pub(super) fn queue_frame(
        &mut self,
        audio: &FrameAudio,
        muted: bool,
        volume: f32,
        throttled: bool,
    ) {
        self.muted = muted;
        self.volume = volume.clamp(0.0, 1.0);
        let next_timing = audio.frame_tstates.is_some();
        if next_timing != self.next_timing {
            self.classic_samples.clear();
            self.next_samples.clear();
            self.next_sample_phase = 0.0;
            self.next_timing = next_timing;
        }
        if muted {
            self.classic_samples.clear();
            self.next_samples.clear();
            self.next_sample_phase = 0.0;
        } else if next_timing {
            self.classic_samples.clear();
            if !throttled {
                self.next_samples.clear();
                self.next_sample_phase = 0.0;
            }
            self.queue_next_frame(audio);
        } else {
            self.next_samples.clear();
            self.queue_classic_frame(audio, throttled);
        }
    }

    fn queue_classic_frame(&mut self, audio: &FrameAudio, throttled: bool) {
        if !throttled {
            self.classic_samples.clear();
        }
        let sample_count = (self.sample_rate / 50).max(1) as usize;
        let mut edge_index = 0;
        for index in 0..sample_count {
            let sample_t = (index as u64 * 69_888 / sample_count as u64) as u32;
            while let Some(&(edge_t, level)) = audio.beeper_edges.get(edge_index) {
                if edge_t > sample_t {
                    break;
                }
                self.level = level;
                edge_index += 1;
            }
            let ay_index = index * audio.ay_samples.len() / sample_count;
            let mono = audio.ay_samples.get(ay_index).copied().unwrap_or(0.5);
            let left = audio.ay_left.get(ay_index).copied().unwrap_or(mono);
            let right = audio.ay_right.get(ay_index).copied().unwrap_or(mono);
            let beep = if self.level { 0.15 } else { -0.15 };
            self.classic_samples
                .push_back((beep + (left - 0.5) * 0.5, beep + (right - 0.5) * 0.5));
        }
        for &(_, level) in audio.beeper_edges.iter().skip(edge_index) {
            self.level = level;
        }
        let max_samples = (self.sample_rate * 3 / 50) as usize;
        let overflow = self
            .classic_samples
            .len()
            .saturating_sub(max_samples.max(1));
        drop(self.classic_samples.drain(..overflow));
    }

    fn queue_next_frame(&mut self, audio: &FrameAudio) {
        let Some(frame_tstates) = audio.frame_tstates else {
            return;
        };
        let sample_count = if audio.ay_samples.is_empty() {
            (frame_tstates as f32 * NEXT_AUDIO_SAMPLE_RATE / 3_500_000.0).round() as usize
        } else {
            audio.ay_samples.len()
        }
        .max(1);
        let max_samples = sample_count.saturating_mul(3);
        let mut edge_index = 0;
        for index in 0..sample_count {
            let sample_t = (index as u64 * u64::from(frame_tstates) / sample_count as u64) as u32;
            while let Some(&(edge_t, level)) = audio.beeper_edges.get(edge_index) {
                if edge_t > sample_t {
                    break;
                }
                self.level = level;
                edge_index += 1;
            }
            let mono = audio.ay_samples.get(index).copied().unwrap_or(0.5);
            let left = audio.ay_left.get(index).copied().unwrap_or(mono);
            let right = audio.ay_right.get(index).copied().unwrap_or(mono);
            let beep = if self.level { 0.15 } else { -0.15 };
            let muted = audio
                .audio_muted_samples
                .get(index)
                .copied()
                .unwrap_or(false);
            self.next_samples.push_back(if muted {
                (0.0, 0.0)
            } else {
                (beep + (left - 0.5) * 0.5, beep + (right - 0.5) * 0.5)
            });
        }
        for &(_, level) in audio.beeper_edges.iter().skip(edge_index) {
            self.level = level;
        }
        let overflow = self.next_samples.len().saturating_sub(max_samples);
        drop(self.next_samples.drain(..overflow));
    }

    fn next_output_sample(&mut self) -> (f32, f32) {
        if !self.next_timing {
            return self.classic_samples.pop_front().unwrap_or((0.0, 0.0));
        }
        let Some(&first) = self.next_samples.front() else {
            self.next_sample_phase = 0.0;
            return (0.0, 0.0);
        };
        let second = self.next_samples.get(1).copied().unwrap_or(first);
        let fraction = self.next_sample_phase;
        let sample = (
            first.0 + (second.0 - first.0) * fraction,
            first.1 + (second.1 - first.1) * fraction,
        );
        self.next_sample_phase += NEXT_AUDIO_SAMPLE_RATE / self.sample_rate.max(1) as f32;
        let consumed = self.next_sample_phase.floor() as usize;
        self.next_sample_phase -= consumed as f32;
        for _ in 0..consumed {
            self.next_samples.pop_front();
        }
        sample
    }
}

pub(super) fn start_beeper(state: Arc<Mutex<BeeperState>>) -> Option<cpal::Stream> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let config = device.default_output_config().ok()?;
    let sample_rate = config.sample_rate().0;
    let channels = config.channels();
    {
        let mut s = state.lock().ok()?;
        s.sample_rate = sample_rate;
        s.channels = channels;
    }
    let stream = device
        .build_output_stream(
            &config.config(),
            move |data: &mut [f32], _| {
                let Ok(mut st) = state.lock() else {
                    return;
                };
                if st.muted {
                    for sample in data.iter_mut() {
                        *sample = 0.0;
                    }
                    return;
                }
                let ch = usize::from(st.channels.max(1));
                for frame in data.chunks_mut(ch) {
                    let (left, right) = st.next_output_sample();
                    let gain = st.volume.clamp(0.0, 1.0);
                    frame[0] = (left * gain).clamp(-1.0, 1.0);
                    if ch > 1 {
                        frame[1] = (right * gain).clamp(-1.0, 1.0);
                    }
                    for s in frame.iter_mut().skip(2) {
                        *s = 0.0;
                    }
                }
            },
            |err| eprintln!("audio error: {err}"),
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}

#[cfg(test)]
mod tests {
    use super::BeeperState;
    use machine::FrameAudio;

    #[test]
    fn switching_timing_modes_clears_the_other_queue() {
        let mut state = BeeperState::default();
        state.queue_frame(&FrameAudio::default(), false, 1.0, true);
        assert_eq!(state.classic_samples.len(), 882);
        state.queue_frame(
            &FrameAudio {
                frame_tstates: Some(70_908),
                ..FrameAudio::default()
            },
            false,
            1.0,
            true,
        );
        assert!(state.classic_samples.is_empty());
        assert_eq!(state.next_samples.len(), 893);
        state.queue_frame(&FrameAudio::default(), false, 1.0, true);
        assert!(state.next_samples.is_empty());
    }

    #[test]
    fn next_ay_samples_remain_queued_across_frame_updates() {
        let mut state = BeeperState {
            sample_rate: 44_100,
            ..BeeperState::default()
        };
        let frame = |value| FrameAudio {
            ay_samples: vec![value; 893],
            ay_left: vec![value; 893],
            ay_right: vec![value; 893],
            frame_tstates: Some(70_908),
            ..FrameAudio::default()
        };

        state.queue_frame(&frame(0.9), false, 1.0, true);
        for _ in 0..882 {
            state.next_output_sample();
        }
        assert_eq!(state.next_samples.len(), 11);

        state.queue_frame(&frame(0.7), false, 1.0, true);
        assert_eq!(state.next_samples.len(), 904);
        for _ in 0..11 {
            let sample = state.next_output_sample();
            assert!((sample.0 - 0.05).abs() < 1e-6);
            assert!((sample.1 - 0.05).abs() < 1e-6);
        }
        assert!((state.next_output_sample().0 + 0.05).abs() < 1e-6);
    }

    #[test]
    fn next_audio_keeps_beeper_edges_with_their_queued_ay_frame() {
        let mut state = BeeperState {
            sample_rate: 44_100,
            ..BeeperState::default()
        };
        let frame = FrameAudio {
            frame_tstates: Some(4),
            audio_muted_samples: Vec::new(),
            beeper_edges: vec![(2, true)],
            ay_samples: vec![0.5; 4],
            ay_left: vec![0.5; 4],
            ay_right: vec![0.5; 4],
        };
        state.queue_frame(&frame, false, 1.0, true);
        assert_eq!(
            state.next_samples,
            [(-0.15, -0.15), (-0.15, -0.15), (0.15, 0.15), (0.15, 0.15)]
        );

        state.queue_frame(
            &FrameAudio {
                frame_tstates: Some(4),
                ay_samples: vec![0.5; 4],
                ay_left: vec![0.5; 4],
                ay_right: vec![0.5; 4],
                ..FrameAudio::default()
            },
            false,
            1.0,
            true,
        );
        assert_eq!(state.next_samples[2], (0.15, 0.15));
    }

    #[test]
    fn next_audio_queue_is_bounded_and_unthrottled_frames_drop_backlog() {
        let mut state = BeeperState::default();
        let frame = || FrameAudio {
            frame_tstates: Some(4),
            ay_samples: vec![0.5; 4],
            ay_left: vec![0.5; 4],
            ay_right: vec![0.5; 4],
            ..FrameAudio::default()
        };
        for _ in 0..5 {
            state.queue_frame(&frame(), false, 1.0, true);
        }
        assert_eq!(state.next_samples.len(), 12);

        state.queue_frame(&frame(), false, 1.0, false);
        assert_eq!(state.next_samples.len(), 4);
    }

    #[test]
    fn next_audio_resamples_to_the_output_device_rate() {
        let mut state = BeeperState {
            sample_rate: 48_000,
            next_timing: true,
            ..BeeperState::default()
        };
        state
            .next_samples
            .extend([(0.0, 0.0), (1.0, 1.0), (0.0, 0.0)]);

        assert_eq!(state.next_output_sample(), (0.0, 0.0));
        let second = state.next_output_sample();
        assert!((second.0 - (44_100.0 / 48_000.0)).abs() < 1e-6);
    }

    #[test]
    fn next_guest_mute_silences_samples_in_the_complete_mix() {
        let mut state = BeeperState::default();
        let frame = FrameAudio {
            frame_tstates: Some(70_908),
            ay_samples: vec![0.9; 8],
            audio_muted_samples: vec![false, true, false, false, false, false, false, false],
            beeper_edges: vec![(0, true)],
            ..FrameAudio::default()
        };
        state.queue_frame(&frame, false, 1.0, true);
        assert!(!state.next_samples.is_empty());

        state.queue_frame(
            &FrameAudio {
                audio_muted_samples: vec![true; 8],
                ..frame
            },
            false,
            1.0,
            true,
        );

        assert_eq!(state.next_samples.len(), 16);
        assert_eq!(state.next_samples[0], (0.35, 0.35));
        assert_eq!(state.next_samples[1], (0.0, 0.0));
        assert_eq!(state.next_samples[2], (0.35, 0.35));
        assert_eq!(state.next_samples[8], (0.0, 0.0));
    }

    #[test]
    fn classic_frames_queue_beeper_and_ay_as_one_timeline() {
        let mut state = BeeperState::default();
        state.queue_frame(
            &FrameAudio {
                ay_samples: vec![0.9; 2],
                beeper_edges: vec![(34_944, true)],
                ..FrameAudio::default()
            },
            false,
            1.0,
            true,
        );
        assert_eq!(state.classic_samples.len(), 882);
        assert!((state.classic_samples[0].0 - 0.05).abs() < 1e-6);
        assert!((state.classic_samples[440].0 - 0.05).abs() < 1e-6);
        assert!((state.classic_samples[441].0 - 0.35).abs() < 1e-6);
        assert_eq!(state.classic_samples[441].0, state.classic_samples[441].1);
        state.queue_frame(&FrameAudio::default(), false, 1.0, false);
        assert_eq!(state.classic_samples.len(), 882);
        assert!((state.next_output_sample().0 - 0.15).abs() < 1e-6);
    }
}
