//! Audio engine: ffmpeg decodes any input (file, radio stream, piped YouTube download) to raw f32 PCM,
//! a reader thread buffers it, and a cpal callback plays it and reports per-channel levels for the meters.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Default)]
struct Shared {
    buf: Mutex<VecDeque<f32>>,
    levels: [AtomicU32; 2],
    paused: AtomicBool,
    volume: AtomicU32,
    frames_played: AtomicU64,
    reader_done: AtomicBool,
    generation: AtomicU64,
    duration_ms: AtomicU64,
    error: Mutex<Option<String>>,
}

pub struct Engine {
    shared: Arc<Shared>,
    _stream: cpal::Stream,
    rate: u32,
    channels: u16,
    /// Seekable source (file or URL); `None` for piped streams.
    input: Option<String>,
    active: bool,
    offset: f64,
    /// ffmpeg audio filter (EQ, loudness), applied from the next play or seek.
    pub filter: Option<String>,
}

impl Engine {
    pub fn new() -> Result<Self, String> {
        let device = cpal::default_host().default_output_device().ok_or("no audio output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.config();
        let shared = Arc::new(Shared::default());
        shared.volume.store(0.7f32.to_bits(), Relaxed);
        let stream = match format {
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, shared.clone()),
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, shared.clone()),
            cpal::SampleFormat::I32 => build::<i32>(&device, &config, shared.clone()),
            _ => build::<f32>(&device, &config, shared.clone()),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self { shared, _stream: stream, rate: config.sample_rate, channels: config.channels, input: None, active: false, offset: 0.0, filter: None })
    }

    /// Start playing `input` (path or URL) from `seek` seconds.
    pub fn play(&mut self, input: &str, seek: f64) {
        self.input = Some(input.to_string());
        self.start(Some(input.to_string()), None, seek);
    }

    /// Play from a byte stream (not seekable); ffmpeg reads it on stdin.
    pub fn play_reader(&mut self, pipe: Box<dyn Read + Send>) {
        self.input = None;
        self.start(None, Some(pipe), 0.0);
    }

    fn start(&mut self, input: Option<String>, pipe: Option<Box<dyn Read + Send>>, seek: f64) {
        let sh = &self.shared;
        let generation = sh.generation.fetch_add(1, Relaxed) + 1;
        sh.buf.lock().unwrap().clear();
        sh.reader_done.store(false, Relaxed);
        sh.frames_played.store(0, Relaxed);
        sh.paused.store(false, Relaxed);
        *sh.error.lock().unwrap() = None;
        if seek == 0.0 {
            sh.duration_ms.store(0, Relaxed);
        }
        self.active = true;
        self.offset = seek;
        let (sh, rate, ch) = (sh.clone(), self.rate, self.channels);
        let filter = self.filter.clone();
        thread::spawn(move || reader(sh, generation, input, pipe, seek, rate, ch, filter));
    }

    pub fn stop(&mut self) {
        self.shared.generation.fetch_add(1, Relaxed);
        self.shared.buf.lock().unwrap().clear();
        self.input = None;
        self.active = false;
    }

    pub fn seek(&mut self, t: f64) {
        if let Some(input) = self.input.clone() {
            self.play(&input, t.max(0.0));
        }
    }

    pub fn set_paused(&self, p: bool) {
        self.shared.paused.store(p, Relaxed);
    }
    pub fn paused(&self) -> bool {
        self.shared.paused.load(Relaxed)
    }
    pub fn set_volume(&self, v: f32) {
        self.shared.volume.store((v * v).to_bits(), Relaxed); // squared: closer to perceived loudness
    }
    pub fn active(&self) -> bool {
        self.active
    }
    pub fn seekable(&self) -> bool {
        self.input.is_some()
    }
    pub fn position(&self) -> f64 {
        self.offset + self.shared.frames_played.load(Relaxed) as f64 / self.rate as f64
    }
    pub fn duration(&self) -> Option<f64> {
        let ms = self.shared.duration_ms.load(Relaxed);
        (ms > 0).then(|| ms as f64 / 1000.0)
    }
    /// True once the decoder finished and every buffered sample was played.
    pub fn ended(&self) -> bool {
        self.active() && self.shared.reader_done.load(Relaxed) && self.shared.buf.lock().unwrap().is_empty()
    }
    pub fn take_error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().take()
    }
    /// RMS of the last audio block, left and right.
    pub fn levels(&self) -> [f32; 2] {
        self.shared.levels.each_ref().map(|l| f32::from_bits(l.load(Relaxed)))
    }
}

fn build<T: SizedSample + FromSample<f32>>(device: &cpal::Device, config: &cpal::StreamConfig, sh: Arc<Shared>) -> Result<cpal::Stream, String> {
    let ch = config.channels as usize;
    device
        .build_output_stream::<T, _, _>(
            *config,
            move |out: &mut [T], _| {
                let paused = sh.paused.load(Relaxed);
                let vol = f32::from_bits(sh.volume.load(Relaxed));
                let mut buf = sh.buf.lock().unwrap();
                let mut frames = 0;
                let mut sq = [0.0f32; 2];
                for frame in out.chunks_mut(ch) {
                    let have = !paused && buf.len() >= ch;
                    for (c, s) in frame.iter_mut().enumerate() {
                        let v = if have { buf.pop_front().unwrap_or(0.0) } else { 0.0 };
                        sq[c.min(1)] += v * v;
                        *s = T::from_sample(v * vol);
                    }
                    frames += have as u64;
                }
                let n = (out.len() / ch).max(1) as f32;
                let mono = ch == 1;
                for (c, l) in sh.levels.iter().enumerate() {
                    let i = if mono { 0 } else { c };
                    l.store((sq[i] / n).sqrt().to_bits(), Relaxed);
                }
                sh.frames_played.fetch_add(frames, Relaxed);
            },
            |e| eprintln!("audio stream error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
fn reader(sh: Arc<Shared>, generation: u64, input: Option<String>, pipe: Option<Box<dyn Read + Send>>, seek: f64, rate: u32, ch: u16, filter: Option<String>) {
    let input = input.unwrap_or_else(|| "pipe:0".into());
    let mut cmd = crate::sources::tool("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "info"]);
    if input.starts_with("http") {
        // some stations and CDNs refuse ffmpeg's default "Lavf" agent with 403
        cmd.args(["-user_agent", "Mozilla/5.0 (X11; Linux x86_64) sonavelt/0.1"]);
        cmd.args(["-reconnect", "1", "-reconnect_streamed", "1", "-reconnect_delay_max", "5"]);
    }
    if seek > 0.0 {
        cmd.args(["-ss", &format!("{seek:.2}")]);
    }
    cmd.args(["-i", &input, "-vn"]);
    if let Some(f) = &filter {
        cmd.args(["-af", f]);
    }
    cmd.args(["-f", "f32le", "-ac", &ch.to_string(), "-ar", &rate.to_string(), "pipe:1"]);
    let stdin = if pipe.is_some() { Stdio::piped() } else { Stdio::null() };
    let mut child = match cmd.stdin(stdin).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            *sh.error.lock().unwrap() = Some(format!("can't start ffmpeg: {e}"));
            sh.reader_done.store(true, Relaxed);
            return;
        }
    };

    if let (Some(mut pipe), Some(mut stdin)) = (pipe, child.stdin.take()) {
        // ends by itself when ffmpeg is killed (broken pipe), which drops and stops the source
        thread::spawn(move || std::io::copy(&mut pipe, &mut stdin));
    }

    // ffmpeg's stderr carries the duration ("Duration: 00:03:21.05") and the last error line.
    let last_err = Arc::new(Mutex::new(String::new()));
    let (sh2, last_err2, stderr) = (sh.clone(), last_err.clone(), child.stderr.take().unwrap());
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(d) = line.trim().strip_prefix("Duration: ").and_then(|s| s.split(',').next())
                && let Some(secs) = parse_hms(d)
                && sh2.generation.load(Relaxed) == generation
            {
                sh2.duration_ms.store((secs * 1000.0) as u64, Relaxed);
            }
            *last_err2.lock().unwrap() = line;
        }
    });

    let mut out = child.stdout.take().unwrap();
    let mut bytes = [0u8; 16384];
    let mut carry = Vec::new();
    let mut total = 0usize;
    let high_water = rate as usize * ch as usize * 2; // buffer ~2s ahead
    loop {
        if sh.generation.load(Relaxed) != generation {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        if sh.buf.lock().unwrap().len() > high_water {
            thread::sleep(Duration::from_millis(20));
            continue;
        }
        let n = match out.read(&mut bytes) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        total += n;
        carry.extend_from_slice(&bytes[..n]);
        let whole = carry.len() / 4 * 4;
        let samples = carry[..whole].as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        if sh.generation.load(Relaxed) == generation {
            sh.buf.lock().unwrap().extend(samples);
        }
        carry.drain(..whole);
    }
    let _ = child.wait();
    if sh.generation.load(Relaxed) == generation {
        if total == 0 {
            let msg = last_err.lock().unwrap().clone();
            *sh.error.lock().unwrap() = Some(if msg.is_empty() { "playback failed".into() } else { msg });
        }
        sh.reader_done.store(true, Relaxed);
    }
}

fn parse_hms(s: &str) -> Option<f64> {
    let mut parts = s.split(':');
    let (h, m, sec) = (parts.next()?, parts.next()?, parts.next()?);
    Some(h.parse::<f64>().ok()? * 3600.0 + m.parse::<f64>().ok()? * 60.0 + sec.parse::<f64>().ok()?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_ffmpeg_duration() {
        assert_eq!(super::parse_hms("00:03:21.50"), Some(201.5));
        assert_eq!(super::parse_hms("N/A"), None);
    }

    // Needs a sound device and a cached YouTube file; run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn engine_plays_and_seeks() {
        let dir = dirs::cache_dir().unwrap().join("sonavelt/youtube");
        let file = std::fs::read_dir(dir).unwrap().next().unwrap().unwrap().path();
        let mut e = super::Engine::new().unwrap();
        e.set_volume(0.0);
        e.play(file.to_str().unwrap(), 0.0);
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(e.position() > 0.5, "position {}", e.position());
        assert!(e.duration().is_some_and(|d| d > 60.0));
        assert!(e.levels()[0] > 0.0);
        e.seek(100.0);
        std::thread::sleep(std::time::Duration::from_millis(800));
        assert!(e.position() > 100.2 && !e.ended(), "position {}", e.position());
    }
}

#[cfg(test)]
mod stream_tests {
    // Network + sound device; run with `cargo test -- --ignored stream`.
    #[test]
    #[ignore]
    fn youtube_stream_starts_fast_and_lands_in_cache() {
        let id = "kJQP7kiw5Fk";
        let cache = dirs::cache_dir().unwrap().join("sonavelt/youtube");
        for f in std::fs::read_dir(&cache).into_iter().flatten().flatten() {
            if f.file_name().to_string_lossy().starts_with(id) {
                let _ = std::fs::remove_file(f.path());
            }
        }
        let mut e = super::Engine::new().unwrap();
        e.set_volume(0.0);
        let t0 = std::time::Instant::now();
        e.play_reader(crate::sources::youtube_stream(id).unwrap());
        while e.position() < 0.05 {
            assert!(t0.elapsed().as_secs() < 20, "no audio after 20s");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        eprintln!("first audio after {:?}", t0.elapsed());
        while crate::sources::youtube_cached(id).is_none() {
            assert!(t0.elapsed().as_secs() < 90, "download never finished");
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        eprintln!("cached after {:?}", t0.elapsed());
    }
}
