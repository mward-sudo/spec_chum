//! Host beeper and AY audio output state.

use std::sync::{Arc, Mutex};

use machine::FrameAudio;

pub(super) struct BeeperState {
    edges: Vec<(u32, bool)>,
    edge_index: usize,
    ay_samples: Vec<f32>,
    ay_left: Vec<f32>,
    ay_right: Vec<f32>,
    ay_index: usize,
    level: bool,
    sample_rate: u32,
    channels: u16,
    frame_t_per_sample: f32,
    t: f32,
    muted: bool,
    /// Linear host output gain 0…1.
    volume: f32,
}

impl Default for BeeperState {
    fn default() -> Self {
        Self {
            edges: Vec::new(),
            edge_index: 0,
            ay_samples: Vec::new(),
            ay_left: Vec::new(),
            ay_right: Vec::new(),
            ay_index: 0,
            level: false,
            sample_rate: 44100,
            channels: 2,
            frame_t_per_sample: 69888.0 / 44100.0,
            t: 0.0,
            muted: false,
            volume: 1.0,
        }
    }
}

impl BeeperState {
    pub(super) fn with_preferences(muted: bool, volume: f32) -> Self {
        Self {
            frame_t_per_sample: 69888.0 / 44100.0,
            muted,
            volume,
            ..Self::default()
        }
    }

    pub(super) fn queue_frame(&mut self, audio: FrameAudio, muted: bool, volume: f32) {
        self.muted = muted;
        self.volume = volume.clamp(0.0, 1.0);
        if !muted {
            self.edges = audio.beeper_edges;
            self.edge_index = 0;
            self.ay_samples = audio.ay_samples;
            self.ay_left = audio.ay_left;
            self.ay_right = audio.ay_right;
            self.ay_index = 0;
            self.t = 0.0;
        }
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
        s.frame_t_per_sample = 69888.0 / (sample_rate as f32 / 50.0);
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
                    while let Some(&(edge_t, level)) = st.edges.get(st.edge_index) {
                        if st.t >= edge_t as f32 {
                            st.level = level;
                            st.edge_index += 1;
                        } else {
                            break;
                        }
                    }
                    let beep = if st.level { 0.15 } else { -0.15 };
                    let (ay_l, ay_r) = if st.ay_index < st.ay_left.len()
                        && st.ay_index < st.ay_right.len()
                    {
                        let l = st.ay_left[st.ay_index];
                        let r = st.ay_right[st.ay_index];
                        st.ay_index += 1;
                        ((l - 0.5) * 0.5, (r - 0.5) * 0.5)
                    } else if st.ay_index < st.ay_samples.len() {
                        let v = st.ay_samples[st.ay_index];
                        st.ay_index += 1;
                        let m = (v - 0.5) * 0.5;
                        (m, m)
                    } else if let (Some(&l), Some(&r)) = (st.ay_left.last(), st.ay_right.last()) {
                        ((l - 0.5) * 0.5, (r - 0.5) * 0.5)
                    } else if let Some(&last) = st.ay_samples.last() {
                        let m = (last - 0.5) * 0.5;
                        (m, m)
                    } else {
                        (0.0, 0.0)
                    };
                    let gain = st.volume.clamp(0.0, 1.0);
                    let left = ((beep + ay_l) * gain).clamp(-1.0, 1.0);
                    let right = ((beep + ay_r) * gain).clamp(-1.0, 1.0);
                    frame[0] = left;
                    if ch > 1 {
                        frame[1] = right;
                    }
                    for s in frame.iter_mut().skip(2) {
                        *s = 0.0;
                    }
                    st.t += st.frame_t_per_sample;
                }
            },
            |err| eprintln!("audio error: {err}"),
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}
