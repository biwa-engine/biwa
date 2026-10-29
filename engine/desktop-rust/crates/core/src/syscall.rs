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
            Self::Wait => "sys_wait",
            Self::CreateObject { .. } => "sys_create_object",
            Self::DeleteObject { .. } => "sys_delete_object",
            Self::AddTransition { .. } => "sys_add_transition",
            Self::StartTransitions { .. } => "sys_start_transitions",
            Self::AwaitTransitions => "sys_await_transitions",
            Self::Sleep { .. } => "sys_sleep",
            Self::UiCreate { .. } => "sys_ui_create",
            Self::UiSetProperty { .. } => "sys_ui_set_property",
            Self::UiSetPropertyWithString { .. } => "sys_ui_set_property_with_string",
            Self::UiPushChild { .. } => "sys_ui_push_child",
            Self::UiSetHandler { .. } => "sys_ui_set_handler",
            Self::EnterScenePage { .. } => "__biwa_enter_scene_page",
        }
    }
}

/// scene を始めるのに要るもの。エンジン (UI) が揃え、vm がそれで scene を始める。
///
/// vm は `on_click(GameWindow::new(canvas, message_area))` で `Game[S]` を作り、
/// ScenePage を見せてから `main_scene(game)` を呼ぶ。関数は番号で指す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneStart {
    pub window: UiId,
    /// SceneStartButton の `on_click` (`fn(GameWindow) -> Game[S]`) の番号。
    pub on_click: u32,
    /// Window の `main_scene` (`fn(Game[S]) -> Game[S]`) の番号。
    pub main_scene: u32,
    pub canvas: UiId,
    pub message_area: UiId,
}
