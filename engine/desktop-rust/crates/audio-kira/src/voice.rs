//! SoundObject 1 つの再生 (kira の `Sound` の自前の実装)。
//!
//! - デコード済みのステレオの PCM を、出力の標本化周波数に合わせて線形補間で読む。
//! - 音量 (dB) と位置 x からの左右の倍率 (パン則は `core::sound::pan_gains`) を掛ける。
//!   値はメインスレッドから原子的に書き換え、こちらで短くならして段差を消す。
//! - ループの区間と回数 (`repeat`。0 は無限) を自分で数える。区間の終わりで区間の始まりへ戻る。
//! - 止めるときは数 ms で絞ってから終わる (ぷつっと切れる音を避ける)。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use biwa_player_core::sound::{DecodedSound, db_to_gain, pan_gains};
use kira::Frame;
use kira::info::Info;
use kira::sound::{Sound, SoundData};

/// 倍率をならす時定数 [s]。
const SMOOTHING_SECS: f64 = 0.01;

/// メインスレッドから再生を操るもの。
pub struct VoiceControl {
    left: AtomicU32,
    right: AtomicU32,
    stop: AtomicBool,
}

impl VoiceControl {
    pub fn new(volume_db: f32, x: f32) -> Self {
        let control = Self { left: AtomicU32::new(0), right: AtomicU32::new(0), stop: AtomicBool::new(false) };
        control.set(volume_db, x);
        control
    }

    pub fn set(&self, volume_db: f32, x: f32) {
        let gain = db_to_gain(volume_db);
        let (l, r) = pan_gains(x);
        self.left.store((gain * l).to_bits(), Ordering::Relaxed);
        self.right.store((gain * r).to_bits(), Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn targets(&self) -> (f32, f32) {
        if self.stop.load(Ordering::Relaxed) {
            (0.0, 0.0)
        } else {
            (f32::from_bits(self.left.load(Ordering::Relaxed)), f32::from_bits(self.right.load(Ordering::Relaxed)))
        }
    }
}

pub struct Voice {
    data: Arc<DecodedSound>,
    control: Arc<VoiceControl>,
    /// 今の位置 [フレーム]。
    position: f64,
    loop_start: f64,
    loop_end: f64,
    repeat: u32,
    /// 区間の終わりに着いた回数。
    passes: u32,
    gain: (f32, f32),
    finished: bool,
}

impl Voice {
    pub fn new(
        data: Arc<DecodedSound>,
        control: Arc<VoiceControl>,
        offset_ms: f64,
        loop_start_ms: f64,
        loop_end_ms: Option<f64>,
        repeat: u32,
    ) -> Self {
        let rate = f64::from(data.sample_rate) / 1000.0;
        let frames = data.frames() as f64;
        let loop_end = loop_end_ms.map_or(frames, |ms| (ms * rate).min(frames));
        let loop_start = (loop_start_ms * rate).min(loop_end);
        let position = offset_ms * rate;
        // 頭から鳴らすなら最初から目標の倍率 (立ち上がりを削らない)。途中からならならして入る。
        let gain = if offset_ms <= 0.0 { control.targets() } else { (0.0, 0.0) };
        let finished = position >= loop_end || loop_end <= 0.0;
        Self { data, control, position, loop_start, loop_end, repeat, passes: 0, gain, finished }
    }

    fn sample(&self, frame: usize) -> (f32, f32) {
        let s = &self.data.samples;
        match s.get(frame * 2..frame * 2 + 2) {
            Some(p) => (p[0], p[1]),
            None => (0.0, 0.0),
        }
    }
}

impl Sound for Voice {
    fn process(&mut self, out: &mut [Frame], dt: f64, _info: &Info) {
        let step = f64::from(self.data.sample_rate) * dt;
        let k = (1.0 - (-dt / SMOOTHING_SECS).exp()) as f32;
        let (tl, tr) = self.control.targets();
        let stopping = self.control.stop.load(Ordering::Relaxed);
        for frame in out.iter_mut() {
            if self.finished {
                *frame = Frame::ZERO;
                continue;
            }
            self.gain.0 += (tl - self.gain.0) * k;
            self.gain.1 += (tr - self.gain.1) * k;
            let i = self.position.floor();
            let t = (self.position - i) as f32;
            let (a, b) = (self.sample(i as usize), self.sample(i as usize + 1));
            let l = a.0 + (b.0 - a.0) * t;
            let r = a.1 + (b.1 - a.1) * t;
            *frame = Frame { left: l * self.gain.0, right: r * self.gain.1 };
            self.position += step;
            if self.position >= self.loop_end {
                self.passes += 1;
                let span = self.loop_end - self.loop_start;
                if (self.repeat == 0 || self.passes < self.repeat) && span > 0.0 {
                    self.position = self.loop_start + (self.position - self.loop_end) % span;
                } else {
                    self.finished = true;
                }
            }
            if stopping && self.gain.0.abs() < 1e-4 && self.gain.1.abs() < 1e-4 {
                self.finished = true;
            }
        }
    }

    fn finished(&self) -> bool {
        self.finished
    }
}

impl SoundData for Voice {
    type Error = std::convert::Infallible;
    type Handle = ();

    fn into_sound(self) -> Result<(Box<dyn Sound>, Self::Handle), Self::Error> {
        Ok((Box::new(self), ()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kira::info::MockInfoBuilder;

    fn data(frames: usize) -> Arc<DecodedSound> {
        // 左は 1、右は 0.5 の直流。
        let samples = (0..frames).flat_map(|_| [1.0, 0.5]).collect();
        Arc::new(DecodedSound { sample_rate: 1000, samples })
    }

    #[test]
    fn plays_repeats_and_finishes() {
        let control = Arc::new(VoiceControl::new(0.0, 0.0));
        // 1000 Hz で 100 フレーム = 100 ms、区間 20..80 ms を 2 回。80 + 60 = 140 ms。
        let mut v = Voice::new(data(100), control, 0.0, 20.0, Some(80.0), 2);
        let info = MockInfoBuilder::new().build();
        let mut out = vec![Frame::ZERO; 139];
        v.process(&mut out, 0.001, &info);
        assert!(!v.finished());
        assert_eq!(out[0], Frame { left: 1.0, right: 0.5 });
        let mut out = vec![Frame::ZERO; 2];
        v.process(&mut out, 0.001, &info);
        assert!(v.finished());
    }

    #[test]
    fn pan_and_volume_are_applied() {
        let control = Arc::new(VoiceControl::new(-6.0206, 1.0));
        let mut v = Voice::new(data(10), control, 0.0, 0.0, None, 1);
        let mut out = vec![Frame::ZERO; 1];
        v.process(&mut out, 0.001, &MockInfoBuilder::new().build());
        assert!(out[0].left.abs() < 1e-6);
        assert!((out[0].right - 0.25).abs() < 1e-3);
    }
}
