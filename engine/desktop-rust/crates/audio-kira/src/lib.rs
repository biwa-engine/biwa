//! 音のミキサーの論理状態 (`core::sound::Mixer`) の写しを、kira で鳴らす。
//!
//! kira に依存するのはこの crate だけで、外から見えるのは [`AudioOutput`] と core の型だけである。
//! ミキサーの意味 (バスの木・バスの中の順序・パン則・フィルタの式・補間) は Biwa が決め、
//! kira はそれを鳴らす部品として使う (`docs/sound.md`)。
//!
//! - 根のバスを出力に繋いだとき (`OpenOutput`) に `AudioManager` を作る。根の effect は main track に挿す
//!   (kira の main track の effect は作るときにしか挿せない)。子のバスは sub track にする。
//! - SoundObject は自前の [`voice::Voice`] (kira の `Sound`) で鳴らす。音量とパン則 (位置 x) を自分で掛け、
//!   ループの区間と回数も自分で数える。
//! - pass filter と duck の音量の段は自前の effect ([`effects`])。compressor (リミッタ) は kira のもの。
//! - 出力デバイスを開けなければ警告して、以後は黙る (論理状態は動き続ける)。

mod effects;
mod voice;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use biwa_player_core::sound::{AudioCommand, DecodedSound, EffectKind, EffectSpec, SoundHandle};
use kira::effect::compressor::{CompressorBuilder, CompressorHandle};
use kira::track::{MainTrackBuilder, TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween};

use effects::{FilterParams, GainParams};
use voice::{Voice, VoiceControl};

/// パラメータの変化をならす時間。フレームごとの段差が聞こえないように。
const SMOOTHING: Duration = Duration::from_millis(20);

/// effect を操るもの。
enum EffectControl {
    Filter(Arc<FilterParams>),
    Compressor(CompressorHandle),
    Gain(Arc<GainParams>),
}

/// バスの kira の実体。根は main track で、ハンドルを持たない。
enum Track {
    Main,
    Sub(Box<TrackHandle>),
}

/// 音を鳴らす側。
#[derive(Default)]
pub struct AudioOutput {
    manager: Option<AudioManager<DefaultBackend>>,
    /// 出力を開けなかった。以後は何も鳴らさない。
    failed: bool,
    tracks: HashMap<u32, Track>,
    /// 子のバス → 親のバス (消すときに部分木を辿る)。
    parents: HashMap<u32, u32>,
    effects: HashMap<u32, EffectControl>,
    voices: HashMap<u32, (u32, Arc<VoiceControl>)>,
    sounds: HashMap<SoundHandle, Arc<DecodedSound>>,
}

impl AudioOutput {
    pub fn new() -> Self {
        Self::default()
    }

    /// デコードした音を受け取る。`Play` より先に届く (エンジンは読めてから鳴らす)。
    pub fn load(&mut self, sound: SoundHandle, data: Arc<DecodedSound>) {
        self.sounds.insert(sound, data);
    }

    /// 論理状態の変化を当てる。
    pub fn apply(&mut self, commands: Vec<AudioCommand>) {
        for command in commands {
            if self.failed {
                return;
            }
            self.apply_one(command);
        }
    }

    fn apply_one(&mut self, command: AudioCommand) {
        match command {
            AudioCommand::OpenOutput { bus, effects, volume_db } => self.open(bus, &effects, volume_db),
            AudioCommand::CreateBus { bus, parent, effects, volume_db } => {
                let mut builder = TrackBuilder::new().volume(decibels(volume_db));
                for spec in &effects {
                    let control = match spec.kind {
                        EffectKind::Compressor => {
                            EffectControl::Compressor(builder.add_effect(compressor_builder(&spec.params)))
                        }
                        _ => {
                            let (effect, control) = own_effect(spec);
                            builder.add_built_effect(effect);
                            control
                        }
                    };
                    self.effects.insert(spec.id, control);
                }
                let handle = match self.tracks.get_mut(&parent) {
                    Some(Track::Sub(parent)) => parent.add_sub_track(builder),
                    Some(Track::Main) => match &mut self.manager {
                        Some(manager) => manager.add_sub_track(builder),
                        None => return,
                    },
                    None => return,
                };
                match handle {
                    Ok(handle) => {
                        self.tracks.insert(bus, Track::Sub(Box::new(handle)));
                        self.parents.insert(bus, parent);
                    }
                    Err(e) => eprintln!("[biwa] failed to create sound bus {bus}: {e}"),
                }
            }
            AudioCommand::DeleteBus { bus } => self.delete(bus),
            AudioCommand::Play { object, sound, bus, offset_ms, loop_start_ms, loop_end_ms, repeat, volume_db, x } => {
                let Some(data) = self.sounds.get(&sound).cloned() else { return };
                let control = Arc::new(VoiceControl::new(volume_db, x));
                let voice = Voice::new(data, control.clone(), offset_ms, loop_start_ms, loop_end_ms, repeat);
                let played = match self.tracks.get_mut(&bus) {
                    Some(Track::Sub(track)) => track.play(voice).map(|_| ()).map_err(|e| e.to_string()),
                    Some(Track::Main) => match &mut self.manager {
                        Some(manager) => manager.main_track().play(voice).map(|_| ()).map_err(|e| e.to_string()),
                        None => return,
                    },
                    None => return,
                };
                match played {
                    Ok(()) => {
                        self.voices.insert(object, (bus, control));
                    }
                    Err(e) => eprintln!("[biwa] failed to play sound object {object}: {e}"),
                }
            }
            AudioCommand::Stop { object } => {
                if let Some((_, control)) = self.voices.remove(&object) {
                    control.stop();
                }
            }
            AudioCommand::SetObject { object, volume_db, x } => {
                if let Some((_, control)) = self.voices.get(&object) {
                    control.set(volume_db, x);
                }
            }
            AudioCommand::SetBus { bus, volume_db } => {
                let tween = Tween { duration: SMOOTHING, ..Default::default() };
                match self.tracks.get_mut(&bus) {
                    Some(Track::Sub(track)) => track.set_volume(decibels(volume_db), tween),
                    Some(Track::Main) => {
                        if let Some(manager) = &mut self.manager {
                            manager.main_track().set_volume(decibels(volume_db), tween);
                        }
                    }
                    None => {}
                }
            }
            AudioCommand::SetEffect { effect, param, value } => match self.effects.get_mut(&effect) {
                Some(EffectControl::Filter(params)) => params.set(param, value),
                Some(EffectControl::Gain(params)) => params.set(value),
                Some(EffectControl::Compressor(handle)) => set_compressor(handle, param, value),
                None => {}
            },
        }
    }

    /// 出力を開き、根のバスを main track にする。
    fn open(&mut self, bus: u32, effects: &[EffectSpec], volume_db: f32) {
        let mut main = MainTrackBuilder::new().volume(decibels(volume_db));
        let mut controls = Vec::new();
        for spec in effects {
            let control = match spec.kind {
                EffectKind::Compressor => EffectControl::Compressor(main.add_effect(compressor_builder(&spec.params))),
                _ => {
                    let (effect, control) = own_effect(spec);
                    main.add_built_effect(effect);
                    control
                }
            };
            controls.push((spec.id, control));
        }
        let settings = AudioManagerSettings { main_track_builder: main, ..Default::default() };
        match AudioManager::<DefaultBackend>::new(settings) {
            Ok(manager) => {
                self.manager = Some(manager);
                self.tracks.insert(bus, Track::Main);
                self.effects.extend(controls);
            }
            Err(e) => {
                eprintln!("[biwa] could not open the audio output ({e}); sound is muted");
                self.failed = true;
            }
        }
    }

    /// バスとその部分木を消す。kira はハンドルを手放したトラックを (中の音ごと) 片付ける。
    fn delete(&mut self, bus: u32) {
        let mut doomed = vec![bus];
        let mut i = 0;
        while i < doomed.len() {
            let parent = doomed[i];
            doomed.extend(self.parents.iter().filter(|(_, p)| **p == parent).map(|(c, _)| *c));
            i += 1;
        }
        for id in &doomed {
            self.parents.remove(id);
            if let Some(Track::Main) = self.tracks.remove(id) {
                // 根を消したら出力ごと閉じる。
                self.manager = None;
            }
        }
        self.voices.retain(|_, (b, _)| !doomed.contains(b));
    }
}

fn decibels(db: f32) -> Decibels {
    Decibels(db.max(Decibels::SILENCE.0))
}

fn own_effect(spec: &EffectSpec) -> (Box<dyn kira::effect::Effect>, EffectControl) {
    match spec.kind {
        EffectKind::PassFilter { steep } => {
            let params = Arc::new(FilterParams::new(spec.params[0], spec.params[1]));
            (Box::new(effects::PassFilter::new(params.clone(), steep)), EffectControl::Filter(params))
        }
        _ => {
            let params = Arc::new(GainParams::new(spec.params.first().copied().unwrap_or(0.0)));
            (Box::new(effects::GainStage::new(params.clone())), EffectControl::Gain(params))
        }
    }
}

fn compressor_builder(params: &[f32]) -> CompressorBuilder {
    let p = |i: usize, default: f32| f64::from(params.get(i).copied().unwrap_or(default));
    CompressorBuilder::new()
        .threshold(p(0, 0.0))
        .ratio(p(1, 1.0))
        .attack_duration(Duration::from_secs_f64(p(2, 10.0).max(0.0) / 1000.0))
        .release_duration(Duration::from_secs_f64(p(3, 100.0).max(0.0) / 1000.0))
        .makeup_gain(Decibels(p(4, 0.0) as f32))
}

fn set_compressor(handle: &mut CompressorHandle, param: u8, value: f32) {
    let tween = Tween { duration: SMOOTHING, ..Default::default() };
    let ms = Duration::from_secs_f64(f64::from(value.max(0.0)) / 1000.0);
    match param {
        0 => handle.set_threshold(f64::from(value), tween),
        1 => handle.set_ratio(f64::from(value), tween),
        2 => handle.set_attack_duration(ms, tween),
        3 => handle.set_release_duration(ms, tween),
        4 => handle.set_makeup_gain(Decibels(value), tween),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use biwa_player_core::sound::{EffectKind, EffectSpec};

    /// 実際の出力デバイスを開いて 0.3 秒鳴らす。デバイスの無い環境では警告して黙ることを確かめる。
    /// `cargo test -p biwa-player-audio-kira -- --ignored` で手元の確認に使う。
    #[test]
    #[ignore = "opens the real audio device"]
    fn plays_a_tone_on_the_default_device() {
        let rate = 48000;
        let samples = (0..rate * 3 / 10)
            .flat_map(|i| {
                let v = (std::f32::consts::TAU * 440.0 * i as f32 / rate as f32).sin() * 0.2;
                [v, v]
            })
            .collect();
        let mut out = AudioOutput::new();
        out.load(SoundHandle(1), Arc::new(DecodedSound { sample_rate: rate as u32, samples }));
        let limiter = EffectSpec { id: 1, kind: EffectKind::Compressor, params: vec![-1.0, 20.0, 1.0, 50.0, 0.0] };
        out.apply(vec![
            AudioCommand::OpenOutput { bus: 1, effects: vec![limiter], volume_db: 0.0 },
            AudioCommand::CreateBus { bus: 2, parent: 1, effects: Vec::new(), volume_db: -3.0 },
            AudioCommand::Play {
                object: 1,
                sound: SoundHandle(1),
                bus: 2,
                offset_ms: 0.0,
                loop_start_ms: 0.0,
                loop_end_ms: None,
                repeat: 1,
                volume_db: 0.0,
                x: -0.5,
            },
        ]);
        eprintln!("output opened: {}", !out.failed);
        std::thread::sleep(Duration::from_millis(500));
        out.apply(vec![AudioCommand::DeleteBus { bus: 1 }]);
    }
}
