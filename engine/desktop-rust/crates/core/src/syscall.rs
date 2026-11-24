//! ゲーム (wasm) からエンジンへの依頼。
//!
//! std の `[[native(arch="wasm")]]` が import する `biwa:engine` の syscall のうち、
//! エンジンに用があるものを、引数を Rust の値に直した形で持つ。
//! 番号や引数の並びは `engine/nodejs/src/engine/vm/wasm/contract.ts` と同じ取り決めで、
//! wasm の値 (externref / anyref) を触るだけのもの (文字列の連結や Vec・Map) は
//! vm の中で済むのでここには来ない。
//!
//! どれを止める (結果を待つ) かは [`Syscall::blocks`] が決める。

/// ui_id。UI Element と canvas オブジェクトの識別子 (どちらも vm が採番する)。
pub type UiId = u32;

/// エンジンへの依頼 1 つ。
#[derive(Debug, Clone, PartialEq)]
pub enum Syscall {
    /// Message Window に content を積む。描画は始まらない。
    ContentPushText {
        ui: UiId,
        text: String,
        /// 秒間何文字出るか。0 以下なら送らずに一度に出す。
        speed: f32,
        /// 0 = vw, 1 = vh。
        size_unit: i32,
        size_value: f32,
        /// 100〜900。
        weight: i32,
        r: i32,
        g: i32,
        b: i32,
        a: i32,
    },
    /// 積まれた content を出し始める。
    ContentFlush { ui: UiId },
    /// 枠を空にする。
    ContentClear { ui: UiId },
    /// クリックが来るまで止まる。
    Wait,

    /// canvas にオブジェクトを置く。`id` は vm が採番済み。
    CreateObject {
        id: UiId,
        canvas: UiId,
        path: String,
        layer: i32,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        alpha: i32,
        theta: f32,
    },
    DeleteObject { id: UiId, after: i32 },
    AddTransition {
        id: UiId,
        param: i32,
        kind: i32,
        val_i: i32,
        val_f: f32,
        after: i32,
        duration: i32,
    },
    StartTransitions { sync: i32 },
    /// sync 印の演出が終わるまで止まる。
    AwaitTransitions,
    /// エンジン時計の上で指定時間止まる。
    Sleep { ms: i32 },

    /// UI Element を作る。`id` は vm が採番済み。
    UiCreate { id: UiId, kind: i32 },
    UiSetProperty {
        id: UiId,
        kind: i32,
        u1: i32,
        u2: i32,
        u3: i32,
        u4: i32,
        i1: i32,
        i2: i32,
        f: f32,
    },
    UiSetPropertyWithString {
        id: UiId,
        kind: i32,
        u: i32,
        i: i32,
        f: f32,
        s: String,
    },
    UiPushChild { parent: UiId, child: UiId },
    /// Element にハンドラを預けた。`handle` は vm の表の番号 (関数そのものは vm に残る)。
    UiSetHandler { id: UiId, kind: i32, handle: u32 },
    /// Element に値を預けた。`handle` は vm の表の番号 (値そのものは vm に残る)。
    UiSetValue { id: UiId, kind: i32, handle: u32 },

    // --- 音 (`docs/sound.md`)。id は vm が SoundObject / Bus / Effect の空間ごとに採番済み。 ---
    /// バスを作る。
    SoundBusCreate { bus: u32 },
    /// バスの effect の列の末尾に、作ってある effect を挿す (繋ぐ前だけ)。
    SoundBusAddEffect { bus: u32, effect: u32 },
    /// effect を作る (まだどのバスにも挿さない)。
    SoundEffectCreate { effect: u32, kind: i32, cfg1: i32, cfg2: i32 },
    SoundBusAttach { bus: u32, parent: u32 },
    /// 根のバスを出力に繋ぐ。
    SoundOutputAttach { bus: u32 },
    SoundBusAddStopAll { bus: u32, after: i32 },
    SoundBusDelete { bus: u32, after: i32 },
    SoundBusAddTransition { bus: u32, param: i32, kind: i32, value: f32, after: i32, duration: i32 },
    SoundBusSetParam { bus: u32, param: i32, value: f32 },
    /// effect にものをぶら下げる (duck の key など)。足した順に index が決まる。使わない引数は 0。
    SoundEffectAddParam { effect: u32, param: i32, i1: i32, i2: i32, f1: f32, f2: f32, f3: f32 },
    /// `index` は、effect に 1 つだけあるパラメータなら 0、ぶら下げたもののパラメータならぶら下げた順。
    SoundEffectAddTransition { effect: u32, param: i32, index: i32, kind: i32, value: f32, after: i32, duration: i32 },
    SoundEffectSetParam { effect: u32, param: i32, index: i32, value: f32 },
    /// SoundObject を作る。まだ鳴らない。`loop_end_ms` = 0 はファイルの終わり。
    SoundCreate {
        id: u32,
        path: String,
        bus: u32,
        volume_db: f32,
        x: f32,
        y: f32,
        loop_start_ms: i32,
        loop_end_ms: i32,
        normalize: i32,
    },
    /// 開始を積む。`repeat` = 0 は無限。
    SoundAddStart { id: u32, after: i32, offset_ms: i32, repeat: i32 },
    SoundAddStop { id: u32, after: i32 },
    SoundAddTransition { id: u32, param: i32, kind: i32, value: f32, after: i32, duration: i32 },
    SoundPreload { path: String },

    /// scene を始めるので ScenePage を見せる (vm 自身が流す。std の import ではない)。
    EnterScenePage { window: UiId },
}

impl Syscall {
    /// 結果を待つ (ゲームを止める) syscall か。
    pub fn blocks(&self) -> bool {
        matches!(self, Self::Wait | Self::AwaitTransitions | Self::Sleep { .. })
    }

    /// 発行した直後にまとめを送り出すべきか。
    ///
    /// 止まらない syscall はまとめて 1 通で送る。遷移の発火だけは待たせたくないので区切る。
    /// 1 通に収まるので、エンジンが発火の途中でフレームを描くことも無い
    /// (`contract.ts` の `FLUSH_AFTER_CAST` と同じ)。
    pub fn flushes_batch(&self) -> bool {
        matches!(self, Self::StartTransitions { .. })
    }

    /// ログに出す短い名前 (std の import 名)。
    pub fn name(&self) -> &'static str {
        match self {
            Self::ContentPushText { .. } => "sys_content_push_text",
            Self::ContentFlush { .. } => "sys_content_flush",
            Self::ContentClear { .. } => "sys_content_clear",
            Self::Wait => "sys_timeline_wait_click",
            Self::CreateObject { .. } => "sys_canvas_object_create",
            Self::DeleteObject { .. } => "sys_canvas_object_delete",
            Self::AddTransition { .. } => "sys_canvas_object_add_transition",
            Self::StartTransitions { .. } => "sys_timeline_fire",
            Self::AwaitTransitions => "sys_timeline_await_synced",
            Self::Sleep { .. } => "sys_timeline_sleep",
            Self::UiCreate { .. } => "sys_ui_create",
            Self::UiSetProperty { .. } => "sys_ui_set_property",
            Self::UiSetPropertyWithString { .. } => "sys_ui_set_property_with_string",
            Self::UiPushChild { .. } => "sys_ui_push_child",
            Self::UiSetHandler { .. } => "sys_ui_set_handler",
            Self::UiSetValue { .. } => "sys_ui_set_value",
            Self::SoundBusCreate { .. } => "sys_sound_bus_create",
            Self::SoundBusAddEffect { .. } => "sys_sound_bus_add_effect",
            Self::SoundEffectCreate { .. } => "sys_sound_effect_create",
            Self::SoundBusAttach { .. } => "sys_sound_bus_attach",
            Self::SoundOutputAttach { .. } => "sys_sound_output_attach",
            Self::SoundBusAddStopAll { .. } => "sys_sound_bus_add_stop_all",
            Self::SoundBusDelete { .. } => "sys_sound_bus_delete",
            Self::SoundBusAddTransition { .. } => "sys_sound_bus_add_transition",
            Self::SoundBusSetParam { .. } => "sys_sound_bus_set_param",
            Self::SoundEffectAddParam { .. } => "sys_sound_effect_add_param",
            Self::SoundEffectAddTransition { .. } => "sys_sound_effect_add_transition",
            Self::SoundEffectSetParam { .. } => "sys_sound_effect_set_param",
            Self::SoundCreate { .. } => "sys_sound_object_create",
            Self::SoundAddStart { .. } => "sys_sound_object_add_start",
            Self::SoundAddStop { .. } => "sys_sound_object_add_stop",
            Self::SoundAddTransition { .. } => "sys_sound_object_add_transition",
            Self::SoundPreload { .. } => "sys_sound_preload",
            Self::EnterScenePage { .. } => "__biwa_enter_scene_page",
        }
    }
}

/// scene を始めるのに要るもの。エンジン (UI) が揃え、vm がそれで scene を始める。
///
/// vm は `on_click(window_handle)` で `Game[S]` を作り、
/// ScenePage を見せてから `main_scene(game)` を呼ぶ。関数と値は番号で指す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneStart {
    pub window: UiId,
    /// SceneStartButton の `on_click` (`fn(WindowHandle) -> Game[S]`) の番号。
    pub on_click: u32,
    /// Window の `main_scene` (`fn(Game[S]) -> Game[S]`) の番号。
    pub main_scene: u32,
    /// Window に預けた std の `WindowHandle` の番号。そのまま `on_click` に渡す。
    /// 出力先 (Canvas / MessageArea) もこの中にあり、エンジンからは返さない。
    pub window_handle: u32,
}
