//! Biwa のゲーム (WasmGC の `game.wasm`) を wasmtime で動かす。
//!
//! wasm は**専用の OS スレッド**で動く (wasmtime の `Store` は `!Sync` なので 1 本に固定する)。
//! エンジンはメインスレッドにいて、両者はチャネルでやり取りする。
//! TypeScript 版の Worker とメインスレッドの分担 (`engine/nodejs/src/engine/vm/wasm/`) と同じ形で、
//! 違うのは輸送路だけである。
//!
//! - 止まらない syscall はまとめて 1 通で送る ([`VmEvent::Batch`])。
//!   ブロッキングの syscall の前と、遷移の発火の後で区切る (発火の原子性。`docs/media-object-model.md`)。
//! - 止まる syscall は [`VmEvent::Call`] で送り、エンジンが [`CallReply`] で返事をするまで
//!   wasm のスレッドが `recv()` で止まる。`Atomics.wait` も SharedArrayBuffer も要らない。
//! - 文字列・Vec・Map の実体は Rust の値として externref に入れる (`values.rs`)。
//!   JS 版では「JS にしか実体を置けない」ために `local` という区分が要ったが、ここでは
//!   wasm のスレッドの中で済むだけで、エンジンには来ない。

mod host;
mod run;
mod values;

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread::JoinHandle;
use std::time::Duration;

use biwa_player_core::syscall::{SceneStart, Syscall};

/// vm からエンジンへの知らせ。
#[derive(Debug)]
pub enum VmEvent {
    /// 止まらない syscall のまとまり。順に当てる。
    Batch(Vec<Syscall>),
    /// 止まる syscall。終わったら返事をすること。返事をするまで wasm は止まっている。
    Call(Syscall, CallReply),
    /// エントリポイント (`fn main()`) が終わり、scene の開始 ([`HostCommand::StartScene`]) を待っている。
    AwaitingScene,
    /// scene が最後まで進んだ。
    Exited(VmStats),
    /// wasm が trap したか、契約違反があった。
    Failed(String),
}

/// 止まる syscall への返事の手段。
#[derive(Debug)]
pub struct CallReply(SyncSender<Result<(), String>>);

impl CallReply {
    /// 終わったので wasm を進める。
    pub fn done(self) {
        // 送れないのは vm がもう終わっているときだけで、そのときは何もしなくてよい。
        let _ = self.0.send(Ok(()));
    }

    /// 失敗した。wasm の側では syscall が trap になる。
    pub fn fail(self, message: impl Into<String>) {
        let _ = self.0.send(Err(message.into()));
    }
}

/// エンジンから vm への指示。
#[derive(Debug, Clone)]
pub enum HostCommand {
    /// scene を始める。
    StartScene(SceneStart),
    /// 預かった関数を手放してよい (持ち主の Element が消えた)。
    Release(Vec<u32>),
}

/// 実行の統計。wasmtime の GC の性能を見る (段 2) のに使う。
#[derive(Debug, Clone, Default)]
pub struct VmStats {
    /// ホスト関数 (import) が呼ばれた回数。
    pub host_calls: u64,
    /// ホスト側で作った文字列の数。
    pub strings_created: u64,
    /// wasm が実行していた時間 (エンジンの返事を待っていた時間を除く)。
    pub running: Duration,
    /// ホスト関数を呼ぶ間に wasm が走り続けた時間の最大 (GC の停止はここに出る)。
    pub max_gap: Duration,
    /// 上の間隔が 1 ms を超えた回数。
    pub gaps_over_1ms: u64,
    /// wasm を compile するのに掛かった時間。
    pub compile: Duration,
}

/// wasmtime の GC の方式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Collector {
    /// wasmtime に任せる (今は遅延参照カウント)。
    #[default]
    Auto,
    /// 遅延参照カウント (deferred reference counting)。循環は回収しない。
    DeferredReferenceCounting,
    /// 何も回収しない。GC の費用を測るときの比較用。
    Null,
}

impl Collector {
    fn to_wasmtime(self) -> wasmtime::Collector {
        match self {
            Self::Auto => wasmtime::Collector::Auto,
            Self::DeferredReferenceCounting => wasmtime::Collector::DeferredReferenceCounting,
            Self::Null => wasmtime::Collector::Null,
        }
    }
}

/// vm の設定。
#[derive(Debug, Clone, Copy, Default)]
pub struct VmOptions {
    pub collector: Collector,
}

/// 動いている vm への窓口。
pub struct Vm {
    events: Receiver<VmEvent>,
    commands: Sender<HostCommand>,
    thread: Option<JoinHandle<()>>,
}

impl Vm {
    /// `wasm` を専用のスレッドで動かし始める。
    ///
    /// `notify` は vm がエンジンへ何かを送るたびに (vm のスレッドから) 呼ばれる。
    /// エンジンのイベントループを起こすのに使う。
    pub fn spawn(
        wasm: Vec<u8>,
        options: VmOptions,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let (event_tx, events) = mpsc::channel();
        let (commands, command_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("biwa-vm".into())
            .spawn(move || run::run(wasm, options, run::Channels::new(event_tx, command_rx, notify)))?;
        Ok(Self { events, commands, thread: Some(thread) })
    }

    /// 届いている知らせを 1 つ取る。無ければ `None`。
    pub fn try_recv(&self) -> Option<VmEvent> {
        self.events.try_recv().ok()
    }

    /// 知らせが届くまで待つ (ウィンドウを持たない動かし方で使う)。vm が終わっていれば `None`。
    pub fn recv_timeout(&self, timeout: Duration) -> Option<VmEvent> {
        match self.events.recv_timeout(timeout) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => None,
        }
    }

    pub fn send(&self, command: HostCommand) {
        // 送れないのは vm が終わったあとで、そのときは知らせが先に届いている。
        let _ = self.commands.send(command);
    }
}

impl Drop for Vm {
    fn drop(&mut self) {
        // wasm のスレッドは返事を待っているかもしれない。チャネルが閉じれば trap して終わるが、
        // 長い計算の途中なら待たされるので、join はしない (プロセスの終了で消える)。
        drop(self.thread.take());
    }
}

/// 返事のチャネルを作る (`run.rs` から使う)。
fn reply_channel() -> (CallReply, Receiver<Result<(), String>>) {
    let (tx, rx) = mpsc::sync_channel(1);
    (CallReply(tx), rx)
}
