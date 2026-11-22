//! 音のミキサーの論理状態 (`docs/sound.md`)。
//!
//! **論理状態が正で、音の実体は写しである。** canvas が biwa 側のパラメータを正として描くたびに射影するのと同じ形で、
//! 時間・遷移・`after`・ダッキングの判定・削除はすべてここで決め、音を鳴らす側 (kira / Web Audio) には
//! その結果を [`AudioCommand`] として渡すだけにする。こうすると、ネイティブと Web で振る舞いが揃い、
//! headless でも同じ論理が走る。
//!
//! - id 空間は SoundObject・Bus・Effect の 3 つ (vm がそれぞれ 1 から採番する)。
//! - バスは木で、根は 1 本だけが出力に繋がる。エンジンは木を保持するだけで、どのバスが何のためかは知らない。
//! - 音量はすべて dB。-60 dB 以下は無音。
//! - パラメータの遷移は canvas と同じ [`Automation`] で動かす。Hz は対数 (log2) に直して持ち、対数の上で補間する。
//! - 開始・停止・stop_all は `start` (= `sys_timeline_fire`) で発火する。

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::automation::Automation;
use crate::transition::Curve;

/// これ以下の dB は無音。フェードの端点にも使う。
pub const SILENCE_DB: f64 = -60.0;

/// 音のファイル 1 つ (resource) の番号。エンジンが振り、読み込みを依頼する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SoundHandle(pub u32);

/// 音のファイルの読み込みの依頼。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundRequest {
    pub sound: SoundHandle,
    /// `assets/` 基準の相対パス。
    pub path: String,
}

/// デコードした音。ステレオに揃えた PCM (左右を交互に並べた f32)。
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedSound {
    pub sample_rate: u32,
    /// `[L, R, L, R, ...]`。
    pub samples: Vec<f32>,
}

impl DecodedSound {
    /// フレーム (左右の組) の数。
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    /// 長さ [ms]。
    pub fn length_ms(&self) -> f64 {
        if self.sample_rate == 0 { 0.0 } else { self.frames() as f64 * 1000.0 / f64::from(self.sample_rate) }
    }
}

/// effect の種類 (`sys_sound_effect_create` の kind)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    /// 帯域制限 (highpass と lowpass の直列)。`steep` なら 24 dB/oct (2 段)、でなければ 12 dB/oct。
    PassFilter { steep: bool },
    /// 圧縮 (リミッタにも使う)。
    Compressor,
    /// ダッキング。鳴らす側から見ると、ただの音量の段 (param 0 = dB) である。
    Duck,
}

/// pass filter のパラメータ。
pub mod pass_filter {
    pub const LOW_CUT: u8 = 0;
    pub const HIGH_CUT: u8 = 1;
    /// これ以下の low_cut は無効 (何も削らない)。
    pub const MIN_HZ: f64 = 20.0;
    /// これ以上の high_cut は無効。
    pub const MAX_HZ: f64 = 20000.0;
}

/// compressor のパラメータ。
pub mod compressor {
    pub const THRESHOLD: u8 = 0;
    pub const RATIO: u8 = 1;
    pub const ATTACK: u8 = 2;
    pub const RELEASE: u8 = 3;
    pub const MAKEUP: u8 = 4;
    pub const COUNT: u8 = 5;
    /// 既定値 (threshold dB, ratio, attack ms, release ms, makeup dB)。
    pub const DEFAULTS: [f64; 5] = [0.0, 1.0, 10.0, 100.0, 0.0];
}

/// duck のパラメータ (論理の側)。鳴らす側には、ここから決めた音量 (dB) だけが行く。
pub mod duck {
    /// 下げる量 [dB]。正でも負でも「その分だけ下げる」と読む。
    pub const AMOUNT: u8 = 0;
    pub const ATTACK: u8 = 1;
    pub const RELEASE: u8 = 2;
    pub const DEFAULTS: [f64; 3] = [0.0, 80.0, 500.0];
    /// 鳴らす側に送るパラメータ: 今の音量 [dB]。
    pub const GAIN: u8 = 0;
}

/// SoundObject のパラメータ。
pub mod object_param {
    pub const VOLUME: u8 = 0;
    pub const X: u8 = 1;
    pub const Y: u8 = 2;
    /// 再生速度。番号だけ予約 (まだ受け付けない)。
    pub const RATE: u8 = 3;
}

/// Bus のパラメータ。
pub mod bus_param {
    pub const VOLUME: u8 = 0;
}

/// 鳴らす側に渡す effect の組み立て。
#[derive(Debug, Clone, PartialEq)]
pub struct EffectSpec {
    pub id: u32,
    pub kind: EffectKind,
    /// 鳴らす側から見たパラメータの初期値 (pass filter は Hz、compressor は 5 つ、duck は今の dB)。
    pub params: Vec<f32>,
}

/// 鳴らす側への指示。論理状態の変化の写しで、順に当てればよい。
#[derive(Debug, Clone, PartialEq)]
pub enum AudioCommand {
    /// 根のバスを出力に繋ぐ (出力を開く)。effect はこのときに決まる。
    OpenOutput { bus: u32, effects: Vec<EffectSpec>, volume_db: f32 },
    /// 子のバスを作る。親は既に作ってある。effect はこのときに決まる。
    CreateBus { bus: u32, parent: u32, effects: Vec<EffectSpec>, volume_db: f32 },
    /// バスとその中のものを消す。
    DeleteBus { bus: u32 },
    /// 鳴らし始める。
    Play {
        object: u32,
        sound: SoundHandle,
        bus: u32,
        /// 再生を始める位置 [ms]。
        offset_ms: f64,
        /// ループの区間 [ms]。`end` が `None` ならファイルの終わり。
        loop_start_ms: f64,
        loop_end_ms: Option<f64>,
        /// 何回通すか。0 は無限。
        repeat: u32,
        volume_db: f32,
        x: f32,
    },
    /// 止めて解放する。
    Stop { object: u32 },
    SetObject { object: u32, volume_db: f32, x: f32 },
    SetBus { bus: u32, volume_db: f32 },
    SetEffect { effect: u32, param: u8, value: f32 },
}

/// パラメータの持ち主。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Target {
    Object(u32),
    Bus(u32),
    Effect(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Parent {
    /// まだ繋がっていない。
    None,
    Bus(u32),
    Output,
}

#[derive(Debug)]
struct Bus {
    parent: Parent,
    children: Vec<u32>,
    effects: Vec<u32>,
    /// 鳴らす側に作ってあるか (出力まで繋がったか)。
    materialized: bool,
    delete_at: Option<f64>,
}

#[derive(Debug)]
struct Effect {
    kind: EffectKind,
    /// 挿したバス。作っただけで挿していない effect は何もしない。
    bus: Option<u32>,
    /// duck の key のバス。
    key: u32,
    /// duck の今の音量 [dB] (attack / release で動く)。
    duck_gain: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ObjectState {
    /// 作っただけ。
    Idle,
    /// 開始の時刻を待っている。
    Scheduled { at: f64, offset: f64, repeat: u32 },
    /// 開始の時刻を過ぎたが、まだ読み込めていない。
    Waiting { offset: f64, repeat: u32 },
    /// 鳴っている。`end` は無限の音なら無限大。
    Playing { end: f64 },
}

#[derive(Debug)]
struct Object {
    sound: SoundHandle,
    bus: u32,
    loop_start: f64,
    /// 0 ならファイルの終わり。
    loop_end: f64,
    state: ObjectState,
    /// 開始を発火した `start` の通し番号 (stop_all が同じ発火で始めた音を巻き込まないため)。
    fired: u64,
    /// 鳴らす側で鳴らし始めたか。
    sounding: bool,
    /// 最後に鳴らす側へ送った (音量, x)。
    sent: (f32, f32),
}

#[derive(Debug, Clone, Copy)]
enum Pending {
    Start { object: u32, after: f64, offset: f64, repeat: u32 },
    Stop { object: u32, after: f64 },
    StopAll { bus: u32, after: f64 },
}

#[derive(Debug, Clone, Copy)]
enum Stop {
    Object(u32),
    /// バスの部分木で、`before` より前の発火で始めた音を止める。
    All { bus: u32, before: u64 },
}

/// sync 印で発火した開始と停止。
#[derive(Debug, Default)]
struct SyncBatch {
    objects: Vec<u32>,
    /// この束の停止 (時刻, 停止)。飛ばしたときはすぐに実行する。
    stops: Vec<(f64, Stop)>,
}

/// ミキサーの全体。時刻はエンジン時計 [ms] を外から受け取る。
#[derive(Debug, Default)]
pub struct Mixer {
    buses: BTreeMap<u32, Bus>,
    effects: BTreeMap<u32, Effect>,
    objects: BTreeMap<u32, Object>,
    /// 出力に繋いだ根。
    root: Option<u32>,
    params: Automation<Target>,
    pending: Vec<Pending>,
    /// 発火した停止 (時刻, 停止)。sync の束の停止は束の側が持つ。
    stops: Vec<(f64, Stop)>,
    sync_batches: Vec<SyncBatch>,
    /// `start` の通し番号。
    fire_seq: u64,
    /// 音のファイルの長さ [ms]。読み込み中は無い。読めなかったものは 0。
    lengths: HashMap<SoundHandle, f64>,
    /// 最後に鳴らす側へ送ったバスの音量・effect のパラメータ。
    sent_bus: HashMap<u32, f32>,
    sent_effect: HashMap<(u32, u8), f32>,
    commands: Vec<AudioCommand>,
    last_now: f64,
}

impl Mixer {
    // --- バス -------------------------------------------------------------

    pub fn bus_create(&mut self, id: u32) -> Result<(), String> {
        if self.buses.contains_key(&id) {
            return Err(format!("sound bus {id} already exists"));
        }
        self.buses.insert(
            id,
            Bus { parent: Parent::None, children: Vec::new(), effects: Vec::new(), materialized: false, delete_at: None },
        );
        self.params.define(Target::Bus(id), bus_param::VOLUME, 0.0);
        Ok(())
    }

    /// effect の列の末尾に、作ってある effect を挿す。繋いだ後は挿せない。1 つの effect は 1 つのバスにしか挿せない。
    pub fn bus_add_effect(&mut self, bus: u32, effect: u32) -> Result<(), String> {
        let b = self.buses.get(&bus).ok_or_else(|| format!("no such sound bus: {bus}"))?;
        if b.parent != Parent::None {
            return Err(format!("sound bus {bus} is already attached; effects must be added before attaching"));
        }
        let e = self.effects.get_mut(&effect).ok_or_else(|| format!("no such sound effect: {effect}"))?;
        if let Some(owner) = e.bus {
            return Err(format!("sound effect {effect} is already in sound bus {owner}; an effect can be in only one bus"));
        }
        e.bus = Some(bus);
        self.buses.get_mut(&bus).unwrap().effects.push(effect);
        Ok(())
    }

    // --- effect -----------------------------------------------------------

    /// effect を作る (まだどのバスにも挿さない)。パラメータは既定値で始まる。
    pub fn effect_create(&mut self, id: u32, kind: i32, cfg1: i32, cfg2: i32) -> Result<(), String> {
        let (kind, key) = match kind {
            1 => (EffectKind::PassFilter { steep: cfg1 == 1 }, 0),
            20 => (EffectKind::Compressor, 0),
            30 => {
                if cfg2 != 0 {
                    return Err(format!("duck mode {cfg2} is not supported (only 0 = activity)"));
                }
                (EffectKind::Duck, cfg1 as u32)
            }
            _ => return Err(format!("unknown sound effect kind: {kind}")),
        };
        if self.effects.contains_key(&id) {
            return Err(format!("sound effect {id} already exists"));
        }
        self.effects.insert(id, Effect { kind, bus: None, key, duck_gain: 0.0 });
        let target = Target::Effect(id);
        match kind {
            EffectKind::PassFilter { .. } => {
                self.params.define(target, pass_filter::LOW_CUT, pass_filter::MIN_HZ.log2());
                self.params.define(target, pass_filter::HIGH_CUT, pass_filter::MAX_HZ.log2());
            }
            EffectKind::Compressor => {
                for (i, v) in compressor::DEFAULTS.iter().enumerate() {
                    self.params.define(target, i as u8, *v);
                }
            }
            EffectKind::Duck => {
                for (i, v) in duck::DEFAULTS.iter().enumerate() {
                    self.params.define(target, i as u8, *v);
                }
            }
        }
        Ok(())
    }

    /// バスを親の下に繋ぐ。
    pub fn bus_attach(&mut self, bus: u32, parent: u32) -> Result<(), String> {
        if bus == parent {
            return Err(format!("sound bus {bus} cannot be its own parent"));
        }
        if !self.buses.contains_key(&parent) {
            return Err(format!("no such sound bus: {parent}"));
        }
        let b = self.buses.get(&bus).ok_or_else(|| format!("no such sound bus: {bus}"))?;
        if b.parent != Parent::None {
            return Err(format!("sound bus {bus} is already attached"));
        }
        // 親の祖先に自分がいれば閉路になる。
        if self.ancestors(parent).contains(&bus) {
            return Err(format!("attaching sound bus {bus} under {parent} would make a cycle"));
        }
        self.buses.get_mut(&bus).unwrap().parent = Parent::Bus(parent);
        self.buses.get_mut(&parent).unwrap().children.push(bus);
        if self.buses[&parent].materialized {
            self.materialize(bus);
        }
        Ok(())
    }

    /// 根のバスを出力に繋ぐ。根は 1 本だけ。
    pub fn output_attach(&mut self, bus: u32) -> Result<(), String> {
        if let Some(root) = self.root {
            return Err(format!(
                "sound bus {root} is already connected to the output; only one root can be connected (ignored bus {bus})"
            ));
        }
        let b = self.buses.get(&bus).ok_or_else(|| format!("no such sound bus: {bus}"))?;
        if b.parent != Parent::None {
            return Err(format!("sound bus {bus} is attached under another bus and cannot be the root"));
        }
        self.buses.get_mut(&bus).unwrap().parent = Parent::Output;
        self.root = Some(bus);
        self.materialize(bus);
        Ok(())
    }

    /// `after` ミリ秒後に、バスとその部分木を消す。
    pub fn bus_delete(&mut self, bus: u32, after: i32, now: f64) -> Result<(), String> {
        let b = self.buses.get_mut(&bus).ok_or_else(|| format!("no such sound bus: {bus}"))?;
        b.delete_at = Some(now + f64::from(after.max(0)));
        Ok(())
    }

    pub fn bus_add_stop_all(&mut self, bus: u32, after: i32) -> Result<(), String> {
        if !self.buses.contains_key(&bus) {
            return Err(format!("no such sound bus: {bus}"));
        }
        self.pending.push(Pending::StopAll { bus, after: f64::from(after.max(0)) });
        Ok(())
    }

    pub fn bus_add_transition(&mut self, bus: u32, param: i32, kind: i32, value: f32, after: i32, duration: i32) -> Result<(), String> {
        if !self.buses.contains_key(&bus) {
            return Err(format!("no such sound bus: {bus}"));
        }
        if param != i32::from(bus_param::VOLUME) {
            return Err(format!("unknown sound bus param: {param}"));
        }
        let kind = Curve::from_raw(kind).ok_or_else(|| format!("unknown transition kind: {kind}"))?;
        self.params.push(Target::Bus(bus), bus_param::VOLUME, kind, f64::from(value), f64::from(after), f64::from(duration));
        Ok(())
    }

    pub fn bus_set_param(&mut self, bus: u32, param: i32, value: f32) -> Result<(), String> {
        if !self.buses.contains_key(&bus) {
            return Err(format!("no such sound bus: {bus}"));
        }
        if param != i32::from(bus_param::VOLUME) {
            return Err(format!("unknown sound bus param: {param}"));
        }
        self.params.set(Target::Bus(bus), bus_param::VOLUME, f64::from(value));
        Ok(())
    }

    /// effect のパラメータの値を、補間の空間 (Hz は log2) に直す。
    fn effect_value(&self, effect: u32, param: i32, value: f32) -> Result<(u8, f64), String> {
        let e = self.effects.get(&effect).ok_or_else(|| format!("no such sound effect: {effect}"))?;
        let param = u8::try_from(param).map_err(|_| format!("unknown sound effect param: {param}"))?;
        if !self.params.contains(Target::Effect(effect), param) {
            return Err(format!("unknown param {param} for sound effect {effect} ({:?})", e.kind));
        }
        let value = f64::from(value);
        Ok(match e.kind {
            EffectKind::PassFilter { .. } => (param, value.clamp(pass_filter::MIN_HZ, pass_filter::MAX_HZ).log2()),
            _ => (param, value),
        })
    }

    pub fn effect_add_transition(&mut self, effect: u32, param: i32, kind: i32, value: f32, after: i32, duration: i32) -> Result<(), String> {
        let (param, value) = self.effect_value(effect, param, value)?;
        let kind = Curve::from_raw(kind).ok_or_else(|| format!("unknown transition kind: {kind}"))?;
        self.params.push(Target::Effect(effect), param, kind, value, f64::from(after), f64::from(duration));
        Ok(())
    }

    pub fn effect_set_param(&mut self, effect: u32, param: i32, value: f32) -> Result<(), String> {
        let (param, value) = self.effect_value(effect, param, value)?;
        self.params.set(Target::Effect(effect), param, value);
        Ok(())
    }

    // --- 再生 -------------------------------------------------------------

    /// SoundObject を作る。まだ鳴らない。`normalize` は段 2 まで使わない。
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &mut self,
        id: u32,
        sound: SoundHandle,
        bus: u32,
        volume_db: f32,
        x: f32,
        y: f32,
        loop_start_ms: i32,
        loop_end_ms: i32,
    ) -> Result<(), String> {
        if self.objects.contains_key(&id) {
            return Err(format!("sound object {id} already exists"));
        }
        if !self.buses.contains_key(&bus) {
            return Err(format!("no such sound bus: {bus}"));
        }
        self.objects.insert(
            id,
            Object {
                sound,
                bus,
                loop_start: f64::from(loop_start_ms.max(0)),
                loop_end: f64::from(loop_end_ms.max(0)),
                state: ObjectState::Idle,
                fired: 0,
                sounding: false,
                sent: (f32::NAN, f32::NAN),
            },
        );
        let target = Target::Object(id);
        self.params.define(target, object_param::VOLUME, f64::from(volume_db));
        self.params.define(target, object_param::X, f64::from(x).clamp(-1.0, 1.0));
        self.params.define(target, object_param::Y, f64::from(y).clamp(-1.0, 1.0));
        Ok(())
    }

    /// 終わった (解放した) SoundObject への操作は黙って無視する。鳴り終わった SE を止めるのはよくあることなので。
    pub fn add_start(&mut self, id: u32, after: i32, offset: i32, repeat: i32) -> Result<(), String> {
        if !self.objects.contains_key(&id) {
            return Ok(());
        }
        self.pending.push(Pending::Start {
            object: id,
            after: f64::from(after.max(0)),
            offset: f64::from(offset.max(0)),
            repeat: repeat.max(0) as u32,
        });
        Ok(())
    }

    pub fn add_stop(&mut self, id: u32, after: i32) -> Result<(), String> {
        if !self.objects.contains_key(&id) {
            return Ok(());
        }
        self.pending.push(Pending::Stop { object: id, after: f64::from(after.max(0)) });
        Ok(())
    }

    pub fn add_transition(&mut self, id: u32, param: i32, kind: i32, value: f32, after: i32, duration: i32) -> Result<(), String> {
        if !self.objects.contains_key(&id) {
            return Ok(());
        }
        let param = match u8::try_from(param) {
            Ok(p @ (object_param::VOLUME | object_param::X | object_param::Y)) => p,
            Ok(object_param::RATE) => return Err("the sound object param `rate` (3) is reserved".into()),
            _ => return Err(format!("unknown sound object param: {param}")),
        };
        let kind = Curve::from_raw(kind).ok_or_else(|| format!("unknown transition kind: {kind}"))?;
        let mut value = f64::from(value);
        if param != object_param::VOLUME {
            value = value.clamp(-1.0, 1.0);
        }
        self.params.push(Target::Object(id), param, kind, value, f64::from(after), f64::from(duration));
        Ok(())
    }

    /// 音のファイルが読めた (`length_ms`)、または読めなかった (`None`。長さ 0 として扱う)。
    pub fn sound_loaded(&mut self, sound: SoundHandle, length_ms: Option<f64>, now: f64) {
        self.lengths.insert(sound, length_ms.unwrap_or(0.0).max(0.0));
        let waiting: Vec<u32> = self
            .objects
            .iter()
            .filter(|(_, o)| o.sound == sound && matches!(o.state, ObjectState::Waiting { .. }))
            .map(|(id, _)| *id)
            .collect();
        for id in waiting {
            if let ObjectState::Waiting { offset, repeat } = self.objects[&id].state {
                self.begin(id, now, offset, repeat);
            }
        }
    }

    /// 積まれた開始・停止・stop_all・遷移を発火する。
    pub fn start(&mut self, sync: bool, now: f64) {
        self.params.start(sync, now);
        let pending = std::mem::take(&mut self.pending);
        if pending.is_empty() {
            return;
        }
        self.fire_seq += 1;
        let seq = self.fire_seq;
        let mut batch = SyncBatch::default();
        for op in pending {
            match op {
                Pending::Start { object, after, offset, repeat } => {
                    let Some(o) = self.objects.get_mut(&object) else { continue };
                    o.state = ObjectState::Scheduled { at: now + after, offset, repeat };
                    o.fired = seq;
                    if sync {
                        batch.objects.push(object);
                    }
                }
                Pending::Stop { object, after } => {
                    let stop = (now + after, Stop::Object(object));
                    if sync { batch.stops.push(stop) } else { self.stops.push(stop) }
                }
                Pending::StopAll { bus, after } => {
                    let stop = (now + after, Stop::All { bus, before: seq });
                    if sync { batch.stops.push(stop) } else { self.stops.push(stop) }
                }
            }
        }
        if sync && (!batch.objects.is_empty() || !batch.stops.is_empty()) {
            self.sync_batches.push(batch);
        }
    }

    // --- sync ---------------------------------------------------------------

    /// sync 印で始めた有限の音・遷移・停止で、終わっていないものがあるか。
    pub fn has_sync(&self) -> bool {
        self.params.has_sync() || self.sync_batches.iter().any(|b| self.batch_alive(b))
    }

    fn batch_alive(&self, batch: &SyncBatch) -> bool {
        !batch.stops.is_empty()
            || batch.objects.iter().any(|id| {
                self.objects.get(id).is_some_and(|o| match o.state {
                    ObjectState::Scheduled { repeat, .. } | ObjectState::Waiting { repeat, .. } => repeat > 0,
                    ObjectState::Playing { end } => end.is_finite(),
                    ObjectState::Idle => false,
                })
            })
    }

    /// クリックで sync 印のものを畳む。畳むものがあったかを返す。
    ///
    /// - 未開始の有限の音は捨て、鳴っている有限の音は止める。
    /// - 未開始の無限の音はすぐ始める。
    /// - 遷移は終値で固定する。束の中の停止は実行する。
    pub fn skip_sync(&mut self, now: f64) -> bool {
        let mut skipped = self.params.skip_sync(now);
        let batches = std::mem::take(&mut self.sync_batches);
        for batch in batches {
            if !self.batch_alive(&batch) {
                continue;
            }
            skipped = true;
            for id in batch.objects {
                let Some(o) = self.objects.get(&id) else { continue };
                match o.state {
                    ObjectState::Scheduled { repeat: 0, offset, .. } => {
                        self.objects.get_mut(&id).unwrap().state = ObjectState::Scheduled { at: now, offset, repeat: 0 };
                    }
                    ObjectState::Scheduled { .. } | ObjectState::Waiting { repeat: 1.., .. } => self.release(id),
                    ObjectState::Playing { end } if end.is_finite() => self.release(id),
                    _ => {}
                }
            }
            for (_, stop) in batch.stops {
                self.run_stop(stop);
            }
        }
        if skipped {
            self.update(now);
        }
        skipped
    }

    // --- 時計 ---------------------------------------------------------------

    /// 時計を `now` まで進める。
    pub fn update(&mut self, now: f64) {
        let dt = (now - self.last_now).max(0.0);
        self.last_now = now;

        // バスの削除。
        let due: Vec<u32> =
            self.buses.iter().filter(|(_, b)| b.delete_at.is_some_and(|t| now >= t)).map(|(id, _)| *id).collect();
        for bus in due {
            if self.buses.contains_key(&bus) {
                self.delete_subtree(bus);
            }
        }

        // 停止 (sync の束のものも時刻が来れば実行する)。
        let stops = std::mem::take(&mut self.stops);
        let (due, rest): (Vec<_>, Vec<_>) = stops.into_iter().partition(|(at, _)| now >= *at);
        self.stops = rest;
        for (_, stop) in due {
            self.run_stop(stop);
        }
        let mut batch_stops = Vec::new();
        for batch in &mut self.sync_batches {
            let (due, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut batch.stops).into_iter().partition(|(at, _)| now >= *at);
            batch.stops = rest;
            batch_stops.extend(due);
        }
        for (_, stop) in batch_stops {
            self.run_stop(stop);
        }

        // 開始と終了。
        let ids: Vec<u32> = self.objects.keys().copied().collect();
        for id in ids {
            match self.objects[&id].state {
                ObjectState::Scheduled { at, offset, repeat } if now >= at => {
                    let sound = self.objects[&id].sound;
                    if self.lengths.contains_key(&sound) {
                        self.begin(id, at, offset, repeat);
                    } else {
                        self.objects.get_mut(&id).unwrap().state = ObjectState::Waiting { offset, repeat };
                    }
                }
                _ => {}
            }
            if let ObjectState::Playing { end } = self.objects[&id].state
                && now >= end
            {
                self.release(id);
            }
        }

        self.params.update(now);
        self.update_ducks(dt);
        self.sync_out();

        let alive: Vec<bool> = self.sync_batches.iter().map(|b| self.batch_alive(b)).collect();
        let mut i = 0;
        self.sync_batches.retain(|_| {
            i += 1;
            alive[i - 1]
        });
    }

    /// 時間で動いているものがあるか (無限の音が鳴っているだけなら動いていない)。
    pub fn is_animating(&self, now: f64) -> bool {
        self.params.is_animating(now)
            || !self.stops.is_empty()
            || !self.sync_batches.is_empty()
            || self.buses.values().any(|b| b.delete_at.is_some())
            || self.objects.values().any(|o| match o.state {
                ObjectState::Scheduled { .. } | ObjectState::Waiting { .. } => true,
                ObjectState::Playing { end } => end.is_finite(),
                ObjectState::Idle => false,
            })
            || self.effects.iter().any(|(id, e)| e.kind == EffectKind::Duck && (e.duck_gain - self.duck_target(*id)).abs() > 1e-6)
    }

    /// 鳴らす側への指示を取り出す。
    pub fn take_commands(&mut self) -> Vec<AudioCommand> {
        std::mem::take(&mut self.commands)
    }

    // --- 内部 ---------------------------------------------------------------

    fn ancestors(&self, bus: u32) -> Vec<u32> {
        let mut out = vec![bus];
        let mut cur = bus;
        while let Some(Parent::Bus(p)) = self.buses.get(&cur).map(|b| b.parent) {
            out.push(p);
            cur = p;
        }
        out
    }

    fn in_subtree(&self, bus: u32, root: u32) -> bool {
        self.ancestors(bus).contains(&root)
    }

    fn volume(&self, target: Target, param: u8) -> f32 {
        self.params.get(target, param).unwrap_or(0.0) as f32
    }

    fn effect_specs(&self, bus: u32) -> Vec<EffectSpec> {
        self.buses[&bus]
            .effects
            .iter()
            .filter_map(|id| {
                let e = self.effects.get(id)?;
                Some(EffectSpec { id: *id, kind: e.kind, params: self.backend_params(*id) })
            })
            .collect()
    }

    /// 鳴らす側から見た effect のパラメータ。
    fn backend_params(&self, id: u32) -> Vec<f32> {
        let e = &self.effects[&id];
        let target = Target::Effect(id);
        match e.kind {
            EffectKind::PassFilter { .. } => [pass_filter::LOW_CUT, pass_filter::HIGH_CUT]
                .iter()
                .map(|p| self.params.get(target, *p).unwrap_or(0.0).exp2() as f32)
                .collect(),
            EffectKind::Compressor => {
                (0..compressor::COUNT).map(|p| self.params.get(target, p).unwrap_or(0.0) as f32).collect()
            }
            EffectKind::Duck => vec![e.duck_gain as f32],
        }
    }

    /// 出力まで繋がったバスとその子孫を、鳴らす側に作る。
    fn materialize(&mut self, bus: u32) {
        let mut stack = vec![bus];
        while let Some(id) = stack.pop() {
            let b = &self.buses[&id];
            if b.materialized {
                continue;
            }
            let effects = self.effect_specs(id);
            let volume_db = self.volume(Target::Bus(id), bus_param::VOLUME);
            let command = match b.parent {
                Parent::Output => AudioCommand::OpenOutput { bus: id, effects, volume_db },
                Parent::Bus(parent) => AudioCommand::CreateBus { bus: id, parent, effects, volume_db },
                Parent::None => continue,
            };
            self.commands.push(command);
            self.sent_bus.insert(id, volume_db);
            for e in &self.buses[&id].effects {
                for (i, v) in self.backend_params(*e).into_iter().enumerate() {
                    self.sent_effect.insert((*e, i as u8), v);
                }
            }
            let b = self.buses.get_mut(&id).unwrap();
            b.materialized = true;
            stack.extend(b.children.iter().rev().copied());
        }
    }

    /// 鳴り始める。読み込み済みであること。
    fn begin(&mut self, id: u32, at: f64, offset: f64, repeat: u32) {
        let o = &self.objects[&id];
        let length = self.lengths.get(&o.sound).copied().unwrap_or(0.0);
        let loop_end = if o.loop_end > 0.0 { o.loop_end.min(length) } else { length };
        let loop_start = o.loop_start.min(loop_end);
        let end = if repeat == 0 {
            // 読めなかった音 (長さ 0) は無限でもすぐ終わる。
            if loop_end - loop_start > 0.0 { f64::INFINITY } else { at }
        } else {
            let first = (loop_end - offset).max(0.0);
            at + first + f64::from(repeat - 1) * (loop_end - loop_start)
        };
        let (sound, bus, materialized) = (o.sound, o.bus, self.buses.get(&o.bus).is_some_and(|b| b.materialized));
        let volume_db = self.volume(Target::Object(id), object_param::VOLUME);
        let x = self.volume(Target::Object(id), object_param::X);
        let o = self.objects.get_mut(&id).unwrap();
        o.state = ObjectState::Playing { end };
        // 長さが 0 (読めなかった) なら鳴らさない。出力に繋がっていないバスでも鳴らさない (時間は進む)。
        if materialized && end > at && length > 0.0 {
            o.sounding = true;
            o.sent = (volume_db, x);
            self.commands.push(AudioCommand::Play {
                object: id,
                sound,
                bus,
                offset_ms: offset,
                loop_start_ms: loop_start,
                loop_end_ms: (o.loop_end > 0.0).then_some(loop_end),
                repeat,
                volume_db,
                x,
            });
        }
    }

    /// 止めて解放する。
    fn release(&mut self, id: u32) {
        if let Some(o) = self.objects.remove(&id)
            && o.sounding
        {
            self.commands.push(AudioCommand::Stop { object: id });
        }
        self.params.remove(Target::Object(id));
    }

    fn run_stop(&mut self, stop: Stop) {
        match stop {
            Stop::Object(id) => {
                if self.objects.get(&id).is_some_and(|o| o.state != ObjectState::Idle) {
                    self.release(id);
                }
            }
            Stop::All { bus, before } => {
                let ids: Vec<u32> = self
                    .objects
                    .iter()
                    .filter(|(_, o)| o.state != ObjectState::Idle && o.fired < before && self.in_subtree(o.bus, bus))
                    .map(|(id, _)| *id)
                    .collect();
                for id in ids {
                    self.release(id);
                }
            }
        }
    }

    fn delete_subtree(&mut self, bus: u32) {
        let mut doomed = Vec::new();
        let mut stack = vec![bus];
        while let Some(id) = stack.pop() {
            if let Some(b) = self.buses.get(&id) {
                stack.extend(b.children.iter().copied());
                doomed.push(id);
            }
        }
        let doomed_set: BTreeSet<u32> = doomed.iter().copied().collect();
        let objects: Vec<u32> =
            self.objects.iter().filter(|(_, o)| doomed_set.contains(&o.bus)).map(|(id, _)| *id).collect();
        for id in objects {
            // 鳴らす側ではバスごと消えるので、止める指示は要らない。
            self.objects.remove(&id);
            self.params.remove(Target::Object(id));
        }
        if self.buses[&bus].materialized {
            self.commands.push(AudioCommand::DeleteBus { bus });
        }
        if let Parent::Bus(parent) = self.buses[&bus].parent
            && let Some(p) = self.buses.get_mut(&parent)
        {
            p.children.retain(|c| *c != bus);
        }
        if self.root == Some(bus) {
            self.root = None;
        }
        for id in doomed {
            if let Some(b) = self.buses.remove(&id) {
                for e in b.effects {
                    self.effects.remove(&e);
                    self.params.remove(Target::Effect(e));
                    self.sent_effect.retain(|(eid, _), _| *eid != e);
                }
            }
            self.params.remove(Target::Bus(id));
            self.sent_bus.remove(&id);
        }
    }

    /// duck の目標の音量 [dB]。key のバスの部分木で鳴っている音があれば下げる。
    fn duck_target(&self, effect: u32) -> f64 {
        let e = &self.effects[&effect];
        let key = e.key;
        let active = self.buses.contains_key(&key)
            && self.objects.values().any(|o| matches!(o.state, ObjectState::Playing { .. }) && self.in_subtree(o.bus, key));
        if active { -self.params.get(Target::Effect(effect), duck::AMOUNT).unwrap_or(0.0).abs() } else { 0.0 }
    }

    fn update_ducks(&mut self, dt: f64) {
        let ids: Vec<u32> = self.effects.iter().filter(|(_, e)| e.kind == EffectKind::Duck).map(|(id, _)| *id).collect();
        for id in ids {
            let target = self.duck_target(id);
            let t = Target::Effect(id);
            let amount = self.params.get(t, duck::AMOUNT).unwrap_or(0.0).abs();
            let current = self.effects[&id].duck_gain;
            let time = if target < current {
                self.params.get(t, duck::ATTACK).unwrap_or(0.0)
            } else {
                self.params.get(t, duck::RELEASE).unwrap_or(0.0)
            };
            // attack / release は「下げ幅の全部を動く時間」。
            let step = if time <= 0.0 || amount <= 0.0 { f64::INFINITY } else { amount / time * dt };
            let next = if (target - current).abs() <= step { target } else { current + step * (target - current).signum() };
            self.effects.get_mut(&id).unwrap().duck_gain = next;
        }
    }

    /// 変わった値を鳴らす側に送る。
    fn sync_out(&mut self) {
        const EPS: f32 = 1e-4;
        let changed = |old: Option<&f32>, new: f32| old.is_none_or(|o| (o - new).abs() > EPS);
        let buses: Vec<u32> = self.buses.iter().filter(|(_, b)| b.materialized).map(|(id, _)| *id).collect();
        for bus in buses {
            let v = self.volume(Target::Bus(bus), bus_param::VOLUME);
            if changed(self.sent_bus.get(&bus), v) {
                self.sent_bus.insert(bus, v);
                self.commands.push(AudioCommand::SetBus { bus, volume_db: v });
            }
            let effects = self.buses[&bus].effects.clone();
            for e in effects {
                for (i, v) in self.backend_params(e).into_iter().enumerate() {
                    let key = (e, i as u8);
                    if changed(self.sent_effect.get(&key), v) {
                        self.sent_effect.insert(key, v);
                        self.commands.push(AudioCommand::SetEffect { effect: e, param: i as u8, value: v });
                    }
                }
            }
        }
        let ids: Vec<u32> = self.objects.iter().filter(|(_, o)| o.sounding).map(|(id, _)| *id).collect();
        for id in ids {
            let v = self.volume(Target::Object(id), object_param::VOLUME);
            let x = self.volume(Target::Object(id), object_param::X);
            let o = self.objects.get_mut(&id).unwrap();
            if (o.sent.0 - v).abs() > EPS || (o.sent.1 - x).abs() > EPS {
                o.sent = (v, x);
                self.commands.push(AudioCommand::SetObject { object: id, volume_db: v, x });
            }
        }
    }
}

/// dB を振幅の倍率にする。-60 dB 以下は 0 (無音)。
pub fn db_to_gain(db: f32) -> f32 {
    if f64::from(db) <= SILENCE_DB { 0.0 } else { 10f32.powf(db / 20.0) }
}

/// 位置 x (-1..1) からステレオの左右の倍率を決める (パン則)。
///
/// ステレオの音のパンは**バランス**の意味で、反対側を絞る。中央では両方 1 で、
/// 片側へ寄せると反対側が cos の曲線で 0 まで下がる。Web でも同じ式を組む。
pub fn pan_gains(x: f32) -> (f32, f32) {
    let x = x.clamp(-1.0, 1.0);
    let quarter = std::f32::consts::FRAC_PI_2;
    let left = if x > 0.0 { (x * quarter).cos() } else { 1.0 };
    let right = if x < 0.0 { (-x * quarter).cos() } else { 1.0 };
    (left, right)
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: SoundHandle = SoundHandle(1);

    /// master (1, compressor) → bgm (2, duck key=3) / voice (3)。
    fn mixer() -> Mixer {
        let mut m = Mixer::default();
        for id in 1..=3 {
            m.bus_create(id).unwrap();
        }
        m.effect_create(1, 20, 0, 0).unwrap();
        m.effect_create(2, 30, 3, 0).unwrap();
        m.bus_add_effect(1, 1).unwrap();
        m.bus_add_effect(2, 2).unwrap();
        m.effect_set_param(2, i32::from(duck::AMOUNT), 12.0).unwrap();
        m.effect_set_param(2, i32::from(duck::ATTACK), 100.0).unwrap();
        m.effect_set_param(2, i32::from(duck::RELEASE), 400.0).unwrap();
        m.bus_attach(2, 1).unwrap();
        m.bus_attach(3, 1).unwrap();
        m.output_attach(1).unwrap();
        m.take_commands();
        m
    }

    fn plays(cmds: &[AudioCommand]) -> Vec<u32> {
        cmds.iter().filter_map(|c| if let AudioCommand::Play { object, .. } = c { Some(*object) } else { None }).collect()
    }

    fn stops(cmds: &[AudioCommand]) -> Vec<u32> {
        cmds.iter().filter_map(|c| if let AudioCommand::Stop { object } = c { Some(*object) } else { None }).collect()
    }

    #[test]
    fn tree_is_materialized_top_down_when_the_root_reaches_the_output() {
        let mut m = Mixer::default();
        m.bus_create(1).unwrap();
        m.bus_create(2).unwrap();
        m.bus_attach(2, 1).unwrap();
        assert!(m.take_commands().is_empty());
        m.output_attach(1).unwrap();
        let cmds = m.take_commands();
        assert!(matches!(cmds[0], AudioCommand::OpenOutput { bus: 1, .. }));
        assert!(matches!(cmds[1], AudioCommand::CreateBus { bus: 2, parent: 1, .. }));
        // 2 本目の根と、繋いだ後の effect は拒む。
        m.bus_create(3).unwrap();
        assert!(m.output_attach(3).is_err());
        m.effect_create(9, 1, 0, 0).unwrap();
        assert!(m.bus_add_effect(2, 9).is_err());
        // 1 つの effect は 1 つのバスにだけ。
        m.bus_create(4).unwrap();
        m.bus_create(5).unwrap();
        m.bus_add_effect(4, 9).unwrap();
        assert!(m.bus_add_effect(5, 9).is_err());
        assert!(m.bus_attach(1, 2).is_err());
    }

    #[test]
    fn finite_sound_plays_for_its_length_and_ends() {
        let mut m = mixer();
        m.sound_loaded(S, Some(1000.0), 0.0);
        m.create(1, S, 3, 0.0, 0.0, 0.0, 0, 0).unwrap();
        m.add_start(1, 100, 0, 2).unwrap();
        m.start(false, 0.0);
        m.update(50.0);
        assert!(plays(&m.take_commands()).is_empty());
        m.update(100.0);
        assert_eq!(plays(&m.take_commands()), vec![1]);
        m.update(2050.0);
        assert!(m.objects.contains_key(&1));
        m.update(2100.0);
        assert_eq!(stops(&m.take_commands()), vec![1]);
        // voice のバスは bgm の duck の key なので、release (400 ms) が終わるまでは動いている。
        assert!(m.is_animating(2100.0));
        m.update(2600.0);
        assert!(!m.is_animating(2600.0));
    }

    #[test]
    fn waits_for_loading_and_counts_as_unfinished() {
        let mut m = mixer();
        m.create(1, S, 3, 0.0, 0.0, 0.0, 0, 0).unwrap();
        m.add_start(1, 0, 0, 1).unwrap();
        m.start(true, 0.0);
        m.update(500.0);
        assert!(m.has_sync());
        m.sound_loaded(S, Some(300.0), 500.0);
        assert_eq!(plays(&m.take_commands()), vec![1]);
        m.update(799.0);
        assert!(m.has_sync());
        m.update(800.0);
        assert!(!m.has_sync());
    }

    #[test]
    fn missing_file_ends_at_once() {
        let mut m = mixer();
        m.sound_loaded(S, None, 0.0);
        m.create(1, S, 3, 0.0, 0.0, 0.0, 0, 0).unwrap();
        m.add_start(1, 0, 0, 1).unwrap();
        m.start(true, 0.0);
        m.update(0.0);
        assert!(plays(&m.take_commands()).is_empty());
        assert!(!m.has_sync());
    }

    #[test]
    fn skip_drops_finite_and_starts_endless() {
        let mut m = mixer();
        m.sound_loaded(S, Some(5000.0), 0.0);
        m.create(1, S, 3, 0.0, 0.0, 0.0, 0, 0).unwrap();
        m.create(2, S, 2, 0.0, 0.0, 0.0, 0, 0).unwrap();
        m.create(3, S, 3, 0.0, 0.0, 0.0, 0, 0).unwrap();
        m.add_start(1, 0, 0, 1).unwrap();
        m.add_start(2, 3000, 0, 0).unwrap();
        m.add_start(3, 4000, 0, 1).unwrap();
        m.start(true, 0.0);
        m.update(10.0);
        m.take_commands();
        assert!(m.skip_sync(10.0));
        let cmds = m.take_commands();
        assert_eq!(stops(&cmds), vec![1]);
        assert_eq!(plays(&cmds), vec![2]);
        assert!(!m.objects.contains_key(&3));
        assert!(!m.has_sync());
    }

    #[test]
    fn stop_all_spares_sounds_started_by_the_same_fire() {
        let mut m = mixer();
        m.sound_loaded(S, Some(5000.0), 0.0);
        m.create(1, S, 3, 0.0, 0.0, 0.0, 0, 0).unwrap();
        m.add_start(1, 0, 0, 1).unwrap();
        m.start(false, 0.0);
        m.update(10.0);
        m.take_commands();
        m.create(2, S, 3, 0.0, 0.0, 0.0, 0, 0).unwrap();
        m.bus_add_stop_all(3, 0).unwrap();
        m.add_start(2, 0, 0, 1).unwrap();
        m.start(false, 10.0);
        m.update(20.0);
        let cmds = m.take_commands();
        assert_eq!(stops(&cmds), vec![1]);
        assert_eq!(plays(&cmds), vec![2]);
    }

    #[test]
    fn ducking_follows_activity_with_attack_and_release() {
        let mut m = mixer();
        m.sound_loaded(S, Some(1000.0), 0.0);
        m.create(1, S, 3, 0.0, 0.0, 0.0, 0, 0).unwrap();
        m.add_start(1, 0, 0, 1).unwrap();
        m.start(false, 0.0);
        m.update(0.0);
        m.update(50.0);
        assert!((m.effects[&2].duck_gain + 6.0).abs() < 1e-6);
        m.update(100.0);
        assert!((m.effects[&2].duck_gain + 12.0).abs() < 1e-6);
        m.update(999.0);
        m.update(1000.0); // 音が終わる
        m.update(1200.0);
        assert!((m.effects[&2].duck_gain + 6.0).abs() < 0.05);
        m.update(1400.0);
        assert!(m.effects[&2].duck_gain.abs() < 1e-6);
        assert!(m.take_commands().iter().any(|c| matches!(c, AudioCommand::SetEffect { effect: 2, param: 0, .. })));
    }

    #[test]
    fn hz_moves_on_a_log_scale_and_volume_in_db() {
        let mut m = Mixer::default();
        m.bus_create(1).unwrap();
        m.effect_create(1, 1, 1, 0).unwrap();
        m.bus_add_effect(1, 1).unwrap();
        m.output_attach(1).unwrap();
        m.take_commands();
        m.effect_add_transition(1, i32::from(pass_filter::HIGH_CUT), 1, 500.0, 0, 1000).unwrap();
        m.bus_add_transition(1, 0, 1, -20.0, 0, 1000).unwrap();
        m.start(false, 0.0);
        m.update(500.0);
        let hz = m.backend_params(1)[1];
        // 20000 と 500 の対数の中点 ≈ 3162 Hz。
        assert!((hz - 3162.3).abs() < 1.0, "{hz}");
        assert!((m.volume(Target::Bus(1), 0) + 10.0).abs() < 1e-4);
    }

    #[test]
    fn deleting_a_bus_removes_its_subtree_and_sounds() {
        let mut m = mixer();
        m.sound_loaded(S, Some(5000.0), 0.0);
        m.create(1, S, 3, 0.0, 0.0, 0.0, 0, 0).unwrap();
        m.add_start(1, 0, 0, 0).unwrap();
        m.start(false, 0.0);
        m.update(0.0);
        m.bus_delete(3, 100, 0.0).unwrap();
        m.take_commands();
        m.update(100.0);
        let cmds = m.take_commands();
        assert!(cmds.contains(&AudioCommand::DeleteBus { bus: 3 }));
        assert!(m.objects.is_empty());
        assert!(m.create(2, S, 3, 0.0, 0.0, 0.0, 0, 0).is_err());
    }

    #[test]
    fn loop_region_sets_the_length_of_repeats() {
        let mut m = mixer();
        m.sound_loaded(S, Some(1000.0), 0.0);
        m.create(1, S, 3, 0.0, 0.0, 0.0, 200, 800).unwrap();
        m.add_start(1, 0, 0, 3).unwrap();
        m.start(false, 0.0);
        m.update(0.0);
        // 800 + 600 * 2 = 2000。
        m.update(1999.0);
        assert!(m.objects.contains_key(&1));
        m.update(2000.0);
        assert!(!m.objects.contains_key(&1));
    }

    #[test]
    fn pan_law_keeps_center_at_unity() {
        assert_eq!(pan_gains(0.0), (1.0, 1.0));
        let (l, r) = pan_gains(1.0);
        assert!(l.abs() < 1e-6 && r == 1.0);
        assert_eq!(db_to_gain(-60.0), 0.0);
        assert!((db_to_gain(-6.0) - 0.501).abs() < 1e-3);
    }
}
