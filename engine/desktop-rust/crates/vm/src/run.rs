//! wasm のスレッドの本体。`engine/nodejs/src/engine/vm/wasm/worker.ts` の `run` と同じ流れで動く。
//!
//! 1. compile して instantiate する (足りない import は名前付きのエラーになる)
//! 2. エントリポイント (`fn main()`) を呼ぶ。UI がここで作られる
//! 3. scene の開始の指示を待つ
//! 4. `on_click(GameWindow::new(canvas, message_area))` で `Game[S]` を作り、
//!    ScenePage を見せてから `main_scene(game)` を呼ぶ

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use biwa_player_core::syscall::Syscall;
use wasmtime::{Config, Engine, Func, Linker, Module, Result, Store, Val};

use crate::host::{self, HostState};
use crate::{HostCommand, VmEvent, VmOptions, VmStats};

/// コンパイラがエントリポイント (`fn main()`) に付ける固定の名前。
const ENTRYPOINT: &str = "__biwa_entrypoint";
/// std が `GameWindow` を組み立てる入口として host export している名前。
const GAME_WINDOW_NEW: &str = "__biwa_std_game_window_new";

pub(crate) struct Channels {
    events: Sender<VmEvent>,
    commands: Receiver<HostCommand>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl Channels {
    pub(crate) fn new(
        events: Sender<VmEvent>,
        commands: Receiver<HostCommand>,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self { events, commands, notify: Arc::new(notify) }
    }
}

pub(crate) fn run(wasm: Vec<u8>, options: VmOptions, channels: Channels) {
    let events = channels.events.clone();
    let notify = channels.notify.clone();
    let event = match run_game(&wasm, &options, channels) {
        Ok(stats) => VmEvent::Exited(stats),
        // `{:?}` で wasm のバックトレースも付ける。
        Err(e) => VmEvent::Failed(format!("{e:?}")),
    };
    let _ = events.send(event);
    notify();
}

fn run_game(wasm: &[u8], options: &VmOptions, channels: Channels) -> Result<VmStats> {
    let mut config = Config::new();
    config.wasm_gc(true).wasm_function_references(true).collector(options.collector.to_wasmtime());
    let engine = Engine::new(&config)?;

    let compile_start = Instant::now();
    let module = Module::new(&engine, wasm)?;
    let compile = compile_start.elapsed();

    let mut linker = Linker::new(&engine);
    host::define(&mut linker)?;
    let mut store = Store::new(&engine, HostState::new(channels.events, channels.notify));
    let instance = linker.instantiate(&mut store, &module)?;

    // 足りないものがあれば、何かを始める前に名前で叱る。
    let entrypoint = export(&instance, &mut store, ENTRYPOINT)?;
    let game_window_new = export(&instance, &mut store, GAME_WINDOW_NEW)?;

    // 1. UI を出す。UI の syscall は止まらないので、ここで流し切っておく。
    invoke(&mut store, entrypoint, &[])?;
    store.data_mut().flush();
    store.data_mut().send(VmEvent::AwaitingScene);

    // 2. scene の開始を待つ。この間 wasm は何も実行していないので、預かった関数を手放してよい。
    let start = loop {
        match channels.commands.recv() {
            Ok(HostCommand::StartScene(start)) => break start,
            Ok(HostCommand::Release(handles)) => {
                for h in handles {
                    store.data_mut().handlers.remove(&h);
                }
            }
            // エンジンが先に終わった。
            Err(_) => return Ok(finish(store, compile)),
        }
    };

    // 3. scene を始める。値 (`GameWindow` / `Game[S]`) は中身を見ずに受け渡す。
    let on_click = handler(&store, start.on_click, "on_click")?;
    let main_scene = handler(&store, start.main_scene, "main_scene")?;
    let window = invoke(
        &mut store,
        game_window_new,
        &[Val::I32(start.canvas as i32), Val::I32(start.message_area as i32)],
    )?;
    let game = invoke(&mut store, on_click, &window)?;
    store.data_mut().cast(Syscall::EnterScenePage { window: start.window });
    invoke(&mut store, main_scene, &game)?;
    store.data_mut().flush();

    Ok(finish(store, compile))
}

fn finish(store: Store<HostState>, compile: std::time::Duration) -> VmStats {
    let mut stats = store.into_data().stats;
    stats.compile = compile;
    stats
}

fn export(instance: &wasmtime::Instance, store: &mut Store<HostState>, name: &str) -> Result<Func> {
    instance
        .get_func(&mut *store, name)
        .ok_or_else(|| wasmtime::format_err!("the generated module does not export `{name}`"))
}

/// 預かった関数を引く。手放されていれば名前を添えて叱る。
fn handler(store: &Store<HostState>, handle: u32, what: &str) -> Result<Func> {
    store
        .data()
        .handlers
        .get(&handle)
        .copied()
        .ok_or_else(|| wasmtime::format_err!("the {what} handler (#{handle}) is not retained"))
}

/// wasm の関数を呼び、戻り値を返す。
fn invoke(store: &mut Store<HostState>, func: Func, params: &[Val]) -> Result<Vec<Val>> {
    let count = func.ty(&*store).results().len();
    let mut results = vec![Val::I32(0); count];
    store.data_mut().start_running();
    let outcome = func.call(&mut *store, params, &mut results);
    store.data_mut().stop_running();
    outcome?;
    Ok(results)
}
