//! cpal playback of HostSession mono PCM.
//!
//! `HostSession` always emits mono f32 @ [`AUDIO_SAMPLE_RATE`] (44100). Opening the
//! device at its default rate (often 48000 on macOS) without resampling under-produces
//! ~78 samples/frame; zero-stuffing a held beeper DC (−0.15) creates a harsh ~50 Hz buzz.
//! Match egui’s intent: play at 44100 when the device supports it.
//!
//! Idle beeper is a constant ±0.15 DC. Silence↔DC edges (intro unlock, mute, brief
//! underrun zeros) sound like occasional blips; a one-pole DC blocker removes the
//! idle offset while preserving beeper edges.

use std::collections::VecDeque;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use spec_chum_host::AUDIO_SAMPLE_RATE;

use crate::camera::CameraLocked;

/// Seconds of *speaker* PCM (post DC-block) to capture once live (opt-in only).
const RECORD_SECS: usize = 10;
const RECORD_SAMPLES: usize = AUDIO_SAMPLE_RATE as usize * RECORD_SECS;

fn audio_capture_enabled() -> bool {
    std::env::var("SPEC_CHUM_ROOM_AUDIO_CAPTURE")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false)
}

/// Shared PCM queue; the cpal stream is held as a non-send resource (not Sync).
#[derive(Resource, Clone, Debug)]
pub struct AudioOut {
    buffer: Arc<Mutex<VecDeque<f32>>>,
    /// Last mono sample delivered while live (held on short underruns).
    last: Arc<Mutex<f32>>,
    /// When false, the callback outputs silence (intro / user mute).
    live: Arc<AtomicBool>,
    /// Soft fade-in samples remaining after going live (avoids residual click).
    fade_in: Arc<AtomicUsize>,
    /// DC-blocker state shared with the callback.
    dc: Arc<Mutex<DcBlock>>,
}

#[derive(Debug, Default, Clone, Copy)]
struct DcBlock {
    x1: f32,
    y1: f32,
}

impl DcBlock {
    /// ~35 Hz high-pass @ 44.1 kHz — kills beeper DC, keeps square edges.
    fn process(&mut self, x: f32) -> f32 {
        const R: f32 = 0.995;
        let y = x - self.x1 + R * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// User mute (M). Independent of intro gating via [`AudioOut::set_live`].
#[derive(Resource, Debug, Default)]
pub struct AudioMuted(pub bool);

/// Keeps the cpal output stream alive on the main thread.
struct AudioStream {
    _stream: Option<cpal::Stream>,
}

impl AudioOut {
    pub fn push_pcm(&self, samples: &[f32]) {
        if !self.live.load(Ordering::Relaxed) {
            return;
        }
        let Ok(mut buf) = self.buffer.lock() else {
            return;
        };
        // Bound latency: drop oldest — clearing the whole queue causes an audible blip.
        const MAX: usize = AUDIO_SAMPLE_RATE as usize / 2; // ~0.5s
        buf.extend(samples.iter().copied());
        if buf.len() > MAX {
            let drop_n = buf.len() - MAX;
            buf.drain(0..drop_n);
        }
    }

    pub fn set_live(&self, live: bool) {
        let was_live = self.live.swap(live, Ordering::Relaxed);
        if was_live == live {
            return;
        }
        if was_live && !live {
            if let Ok(mut buf) = self.buffer.lock() {
                buf.clear();
            }
            if let Ok(mut last) = self.last.lock() {
                *last = 0.0;
            }
            if let Ok(mut dc) = self.dc.lock() {
                dc.reset();
            }
            self.fade_in.store(0, Ordering::Relaxed);
        } else if !was_live && live {
            // ~15 ms fade after unlock / unmute.
            self.fade_in
                .store((AUDIO_SAMPLE_RATE as usize) / 64, Ordering::Relaxed);
            if let Ok(mut dc) = self.dc.lock() {
                dc.reset();
            }
        }
    }

    pub fn clear(&self) {
        if let Ok(mut buf) = self.buffer.lock() {
            buf.clear();
        }
        if let Ok(mut last) = self.last.lock() {
            *last = 0.0;
        }
        if let Ok(mut dc) = self.dc.lock() {
            dc.reset();
        }
    }
}

#[derive(Debug, Default)]
pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        let (out, stream) = start_audio();
        app.insert_resource(out)
            .init_resource::<AudioMuted>()
            .insert_non_send(stream)
            .add_systems(Update, gate_audio_on_lock);
    }
}

fn gate_audio_on_lock(
    locked: Option<Res<CameraLocked>>,
    muted: Res<AudioMuted>,
    audio: Res<AudioOut>,
) {
    // Silence until sofa cam locks; honour user mute after that.
    let live = locked.is_some() && !muted.0;
    audio.set_live(live);
}

fn start_audio() -> (AudioOut, AudioStream) {
    let buffer = Arc::new(Mutex::new(VecDeque::<f32>::new()));
    let last = Arc::new(Mutex::new(0.0f32));
    let live = Arc::new(AtomicBool::new(false));
    let fade_in = Arc::new(AtomicUsize::new(0));
    let dc = Arc::new(Mutex::new(DcBlock::default()));
    let capture = audio_capture_enabled();
    let record = Arc::new(Mutex::new(if capture {
        Some(Vec::with_capacity(RECORD_SAMPLES))
    } else {
        None
    }));
    let wav_tx = if capture {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(1);
        std::thread::Builder::new()
            .name("living_room-wav".into())
            .spawn(move || {
                if let Ok(samples) = rx.recv() {
                    let path = PathBuf::from("/tmp/spec-chum-room-10s.wav");
                    match write_wav_mono_f32(&path, AUDIO_SAMPLE_RATE, &samples) {
                        Ok(()) => eprintln!(
                            "living_room: wrote {} ({} samples, {RECORD_SECS}s speaker PCM @ {} Hz)",
                            path.display(),
                            samples.len(),
                            AUDIO_SAMPLE_RATE
                        ),
                        Err(e) => {
                            eprintln!("living_room: failed to write {}: {e}", path.display());
                        }
                    }
                }
            })
            .ok();
        Some(tx)
    } else {
        None
    };
    let buf_cb = Arc::clone(&buffer);
    let last_cb = Arc::clone(&last);
    let live_cb = Arc::clone(&live);
    let fade_cb = Arc::clone(&fade_in);
    let dc_cb = Arc::clone(&dc);
    let rec_cb = Arc::clone(&record);
    let wav_cb = wav_tx.clone();
    let out = AudioOut {
        buffer: Arc::clone(&buffer),
        last,
        live,
        fade_in,
        dc,
    };

    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        eprintln!("living_room: no default audio output device");
        return (out, AudioStream { _stream: None });
    };
    let Ok(default_config) = device.default_output_config() else {
        return (out, AudioStream { _stream: None });
    };

    // HostSession PCM is fixed @ 44100. Prefer that rate so we do not underrun
    // against a 48 kHz default (macOS) and zero-stuff held beeper DC into buzz.
    let mut stream_config = default_config.config();
    let play_rate = if output_rate_supported(&device, AUDIO_SAMPLE_RATE, default_config.channels())
    {
        stream_config.sample_rate = cpal::SampleRate(AUDIO_SAMPLE_RATE);
        AUDIO_SAMPLE_RATE
    } else {
        eprintln!(
            "living_room: device lacks {} Hz output; using {} Hz (expect pitch/underrun artefacts)",
            AUDIO_SAMPLE_RATE,
            default_config.sample_rate().0
        );
        default_config.sample_rate().0
    };
    let channels = stream_config.channels as usize;
    let err_fn = |e| eprintln!("living_room audio error: {e}");
    let stream = match default_config.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &stream_config,
            move |data: &mut [f32], _| {
                fill_output(
                    data,
                    channels,
                    &buf_cb,
                    &last_cb,
                    &live_cb,
                    &fade_cb,
                    &dc_cb,
                    &rec_cb,
                    wav_cb.as_ref(),
                );
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            &stream_config,
            move |data: &mut [i16], _| {
                let mut tmp = vec![0.0f32; data.len()];
                fill_output(
                    &mut tmp,
                    channels,
                    &buf_cb,
                    &last_cb,
                    &live_cb,
                    &fade_cb,
                    &dc_cb,
                    &rec_cb,
                    wav_cb.as_ref(),
                );
                for (out_s, s) in data.iter_mut().zip(tmp.iter()) {
                    *out_s = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
                }
            },
            err_fn,
            None,
        ),
        _ => {
            eprintln!("living_room: unsupported audio sample format");
            return (out, AudioStream { _stream: None });
        }
    };

    match stream {
        Ok(stream) => {
            if capture {
                eprintln!(
                    "living_room: audio stream {play_rate} Hz, {channels} ch (HostSession PCM @ {AUDIO_SAMPLE_RATE} Hz); DC-block on; SPEC_CHUM_ROOM_AUDIO_CAPTURE → /tmp/spec-chum-room-10s.wav after {RECORD_SECS}s"
                );
            } else {
                eprintln!(
                    "living_room: audio stream {play_rate} Hz, {channels} ch (HostSession PCM @ {AUDIO_SAMPLE_RATE} Hz); DC-block on"
                );
            }
            if let Err(e) = stream.play() {
                eprintln!("living_room: stream.play() failed: {e}");
                return (out, AudioStream { _stream: None });
            }
            (
                out,
                AudioStream {
                    _stream: Some(stream),
                },
            )
        }
        Err(e) => {
            eprintln!("living_room: failed to start audio stream: {e}");
            (out, AudioStream { _stream: None })
        }
    }
}

fn output_rate_supported(device: &cpal::Device, rate: u32, channels: u16) -> bool {
    let Ok(mut configs) = device.supported_output_configs() else {
        return false;
    };
    configs.any(|c| {
        c.channels() == channels && c.min_sample_rate().0 <= rate && c.max_sample_rate().0 >= rate
    })
}

#[allow(clippy::too_many_arguments)]
fn fill_output(
    data: &mut [f32],
    channels: usize,
    buffer: &Arc<Mutex<VecDeque<f32>>>,
    last: &Arc<Mutex<f32>>,
    live: &Arc<AtomicBool>,
    fade_in: &Arc<AtomicUsize>,
    dc: &Arc<Mutex<DcBlock>>,
    record: &Arc<Mutex<Option<Vec<f32>>>>,
    wav_tx: Option<&SyncSender<Vec<f32>>>,
) {
    // Intro / mute → hard silence.
    if !live.load(Ordering::Relaxed) {
        for s in data.iter_mut() {
            *s = 0.0;
        }
        return;
    }
    let Ok(mut buf) = buffer.lock() else {
        for s in data.iter_mut() {
            *s = 0.0;
        }
        return;
    };
    let Ok(mut hold) = last.lock() else {
        for s in data.iter_mut() {
            *s = 0.0;
        }
        return;
    };
    let Ok(mut dc_state) = dc.lock() else {
        for s in data.iter_mut() {
            *s = 0.0;
        }
        return;
    };
    let fade_total = (AUDIO_SAMPLE_RATE as usize) / 64;
    let mut mono_out = Vec::with_capacity(data.len() / channels.max(1));
    for frame in data.chunks_mut(channels) {
        // Hold last sample on short underruns — stuffing 0 into beeper DC (±0.15)
        // sounds like hash; matched-rate playback should rarely hit this.
        let sample = buf.pop_front().unwrap_or(*hold);
        *hold = sample;
        let mut out = dc_state.process(sample);
        let fade_left = fade_in.load(Ordering::Relaxed);
        if fade_left > 0 {
            let g = 1.0 - (fade_left as f32 / fade_total as f32);
            fade_in.fetch_sub(1, Ordering::Relaxed);
            out *= g.clamp(0.0, 1.0);
        }
        mono_out.push(out);
        for ch in frame.iter_mut() {
            *ch = out;
        }
    }
    if let Ok(mut rec) = record.lock() {
        if let Some(cap) = rec.as_mut() {
            if cap.len() < RECORD_SAMPLES {
                let room = RECORD_SAMPLES - cap.len();
                cap.extend(mono_out.iter().copied().take(room));
                if cap.len() >= RECORD_SAMPLES {
                    // Hand off to a helper thread — never write files on the RT callback.
                    let samples = std::mem::take(cap);
                    *rec = None;
                    if let Some(tx) = wav_tx {
                        let _ = tx.try_send(samples);
                    }
                }
            }
        }
    }
}

fn write_wav_mono_f32(path: &PathBuf, sample_rate: u32, samples: &[f32]) -> std::io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut file = BufWriter::new(file);
    let n = samples.len() as u32;
    let data_bytes = n * 2; // i16 PCM
    let file_size = 36 + data_bytes;
    file.write_all(b"RIFF")?;
    file.write_all(&file_size.to_le_bytes())?;
    file.write_all(b"WAVE")?;
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?; // PCM chunk size
    file.write_all(&1u16.to_le_bytes())?; // PCM
    file.write_all(&1u16.to_le_bytes())?; // mono
    file.write_all(&sample_rate.to_le_bytes())?;
    let byte_rate = sample_rate * 2;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?; // block align
    file.write_all(&16u16.to_le_bytes())?; // bits
    file.write_all(b"data")?;
    file.write_all(&data_bytes.to_le_bytes())?;
    for &s in samples {
        let i = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        file.write_all(&i.to_le_bytes())?;
    }
    file.flush()?;
    Ok(())
}
