//! 自前の effect (kira の `Effect` の実装)。式は Biwa が決め、Web でも同じものを組む (`docs/sound.md`)。
//!
//! - [`PassFilter`]: highpass (low_cut) → lowpass (high_cut) の直列。双二次 (RBJ の式、Q = 0.707) を
//!   12 dB/oct なら 1 段、24 dB/oct なら 2 段。20 Hz 以下の low_cut と 20 kHz 以上の high_cut は素通し。
//! - [`GainStage`]: duck の音量の段。論理状態が決めた dB を掛けるだけ。

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use biwa_player_core::sound::{db_to_gain, pass_filter};
use kira::Frame;
use kira::effect::Effect;
use kira::info::Info;

/// pass filter の Q (Butterworth。cutoff の手前で盛り上がらない)。
const Q: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// pass filter のパラメータ (メインスレッドから書き換える)。
pub struct FilterParams {
    low: AtomicU32,
    high: AtomicU32,
}

impl FilterParams {
    pub fn new(low: f32, high: f32) -> Self {
        Self { low: AtomicU32::new(low.to_bits()), high: AtomicU32::new(high.to_bits()) }
    }

    pub fn set(&self, param: u8, value: f32) {
        match param {
            pass_filter::LOW_CUT => self.low.store(value.to_bits(), Ordering::Relaxed),
            pass_filter::HIGH_CUT => self.high.store(value.to_bits(), Ordering::Relaxed),
            _ => {}
        }
    }

    fn get(&self) -> (f32, f32) {
        (f32::from_bits(self.low.load(Ordering::Relaxed)), f32::from_bits(self.high.load(Ordering::Relaxed)))
    }
}

/// 双二次フィルタの係数 (a0 で割ったもの)。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Coefficients {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coefficients {
    /// RBJ の Audio EQ Cookbook の lowpass / highpass。Web Audio の `BiquadFilterNode` と同じ式。
    fn new(highpass: bool, hz: f64, sample_rate: f64) -> Self {
        let w0 = std::f64::consts::TAU * (hz / sample_rate).min(0.49);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * Q);
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = if highpass {
            ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0)
        } else {
            ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0)
        };
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: -2.0 * cos / a0, a2: (1.0 - alpha) / a0 }
    }
}

/// 双二次フィルタ 1 段の状態 (左右)。
#[derive(Clone, Copy, Debug, Default)]
struct Biquad {
    x1: [f64; 2],
    x2: [f64; 2],
    y1: [f64; 2],
    y2: [f64; 2],
}

impl Biquad {
    fn process(&mut self, c: &Coefficients, ch: usize, x: f64) -> f64 {
        let y = c.b0 * x + c.b1 * self.x1[ch] + c.b2 * self.x2[ch] - c.a1 * self.y1[ch] - c.a2 * self.y2[ch];
        self.x2[ch] = self.x1[ch];
        self.x1[ch] = x;
        self.y2[ch] = self.y1[ch];
        self.y1[ch] = y;
        y
    }
}

pub struct PassFilter {
    params: Arc<FilterParams>,
    stages: usize,
    sample_rate: f64,
    applied: (f32, f32),
    high_pass: Option<Coefficients>,
    low_pass: Option<Coefficients>,
    hp_state: [Biquad; 2],
    lp_state: [Biquad; 2],
}

impl PassFilter {
    pub fn new(params: Arc<FilterParams>, steep: bool) -> Self {
        Self {
            params,
            stages: if steep { 2 } else { 1 },
            sample_rate: 48000.0,
            applied: (f32::NAN, f32::NAN),
            high_pass: None,
            low_pass: None,
            hp_state: Default::default(),
            lp_state: Default::default(),
        }
    }

    fn refresh(&mut self) {
        let (low, high) = self.params.get();
        if (low, high) == self.applied {
            return;
        }
        self.applied = (low, high);
        self.high_pass = (f64::from(low) > pass_filter::MIN_HZ)
            .then(|| Coefficients::new(true, f64::from(low), self.sample_rate));
        self.low_pass = (f64::from(high) < pass_filter::MAX_HZ)
            .then(|| Coefficients::new(false, f64::from(high), self.sample_rate));
    }
}

impl Effect for PassFilter {
    fn init(&mut self, sample_rate: u32, _internal_buffer_size: usize) {
        self.sample_rate = f64::from(sample_rate);
        self.applied = (f32::NAN, f32::NAN);
    }

    fn on_change_sample_rate(&mut self, sample_rate: u32) {
        self.init(sample_rate, 0);
    }

    fn on_start_processing(&mut self) {
        self.refresh();
    }

    fn process(&mut self, input: &mut [Frame], _dt: f64, _info: &Info) {
        for frame in input.iter_mut() {
            let mut s = [f64::from(frame.left), f64::from(frame.right)];
            for (ch, v) in s.iter_mut().enumerate() {
                if let Some(c) = &self.high_pass {
                    for stage in &mut self.hp_state[..self.stages] {
                        *v = stage.process(c, ch, *v);
                    }
                }
                if let Some(c) = &self.low_pass {
                    for stage in &mut self.lp_state[..self.stages] {
                        *v = stage.process(c, ch, *v);
                    }
                }
            }
            *frame = Frame { left: s[0] as f32, right: s[1] as f32 };
        }
    }
}

/// duck の音量の段のパラメータ (dB)。
pub struct GainParams {
    db: AtomicU32,
}

impl GainParams {
    pub fn new(db: f32) -> Self {
        Self { db: AtomicU32::new(db.to_bits()) }
    }

    pub fn set(&self, db: f32) {
        self.db.store(db.to_bits(), Ordering::Relaxed);
    }
}

pub struct GainStage {
    params: Arc<GainParams>,
    gain: f32,
}

impl GainStage {
    pub fn new(params: Arc<GainParams>) -> Self {
        let gain = db_to_gain(f32::from_bits(params.db.load(Ordering::Relaxed)));
        Self { params, gain }
    }
}

impl Effect for GainStage {
    fn process(&mut self, input: &mut [Frame], dt: f64, _info: &Info) {
        let target = db_to_gain(f32::from_bits(self.params.db.load(Ordering::Relaxed)));
        let k = (1.0 - (-dt / 0.01).exp()) as f32;
        for frame in input.iter_mut() {
            self.gain += (target - self.gain) * k;
            *frame = Frame { left: frame.left * self.gain, right: frame.right * self.gain };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kira::info::MockInfoBuilder;

    fn sine(hz: f64, rate: f64, n: usize) -> Vec<Frame> {
        (0..n)
            .map(|i| {
                let v = (std::f64::consts::TAU * hz * i as f64 / rate).sin() as f32;
                Frame { left: v, right: v }
            })
            .collect()
    }

    fn rms(frames: &[Frame]) -> f32 {
        (frames.iter().map(|f| f.left * f.left).sum::<f32>() / frames.len() as f32).sqrt()
    }

    #[test]
    fn lowpass_cuts_highs_and_passes_lows() {
        let info = MockInfoBuilder::new().build();
        let params = Arc::new(FilterParams::new(20.0, 500.0));
        let mut f = PassFilter::new(params.clone(), true);
        f.init(48000, 128);
        f.on_start_processing();
        let mut high = sine(8000.0, 48000.0, 4800);
        f.process(&mut high, 1.0 / 48000.0, &info);
        assert!(rms(&high[2400..]) < 0.01, "{}", rms(&high[2400..]));
        let mut f = PassFilter::new(params, true);
        f.init(48000, 128);
        f.on_start_processing();
        let mut low = sine(100.0, 48000.0, 9600);
        f.process(&mut low, 1.0 / 48000.0, &info);
        assert!(rms(&low[4800..]) > 0.65);
    }

    #[test]
    fn open_filter_is_transparent() {
        let mut f = PassFilter::new(Arc::new(FilterParams::new(20.0, 20000.0)), false);
        f.init(48000, 128);
        f.on_start_processing();
        let mut s = sine(1000.0, 48000.0, 64);
        let before = s.clone();
        f.process(&mut s, 1.0 / 48000.0, &MockInfoBuilder::new().build());
        assert_eq!(s, before);
    }
}
