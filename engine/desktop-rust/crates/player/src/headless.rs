//! ウィンドウを出さずに動かす (段 1・段 2 の確認と計測、CI 用)。
//!
//! クリック待ちのたびに Message Window の文字をログに出し、自動でクリックして進める。
//! sleep はエンジン時計をその分だけ進めて終わらせる (実時間は待たない)。

use std::time::{Duration, Instant};

use biwa_player_core::engine::{CallStatus, Engine};
use biwa_player_core::syscall::Syscall;
use biwa_player_vm::{HostCommand, Vm, VmEvent, VmOptions, VmStats};

use crate::options::Options;

/// vm から何も来ないまま待つ上限。無限ループのシーンで止まらないための保険。
const IDLE_LIMIT: Duration = Duration::from_secs(120);

pub fn run(options: &Options, wasm: Vec<u8>) -> Result<(), String> {
    let started = Instant::now();
    let vm = Vm::spawn(wasm, VmOptions { collector: options.collector }, || {})
        .map_err(|e| format!("failed to start the vm thread: {e}"))?;
    let mut engine = Engine::new();
    let mut waits = 0u32;

    loop {
        let event = vm
            .recv_timeout(IDLE_LIMIT)
            .ok_or_else(|| format!("the game did not respond for {} s", IDLE_LIMIT.as_secs()))?;
        match event {
            VmEvent::Batch(calls) => {
                for call in calls {
                    trace(options, &call);
                    if let Err(e) = engine.apply(call) {
                        eprintln!("[biwa] {e}");
                    }
                }
            }
            VmEvent::Call(call, reply) => {
                trace(options, &call);
                if engine.begin_call(&call) == CallStatus::Pending {
                    drive(&mut engine, &call, &mut waits);
                }
                reply.done();
            }
            VmEvent::AwaitingScene => {
                let start = engine.first_scene_start().map_err(|e| e.to_string())?;
                println!("[biwa] starting the scene (Window {})", start.window);
                vm.send(HostCommand::StartScene(start));
            }
            VmEvent::Exited(stats) => {
                report(&stats, waits, started.elapsed());
                return Ok(());
            }
            VmEvent::Failed(message) => return Err(message),
        }
    }
}

/// 止まっている syscall を、人の代わりに終わらせる。
fn drive(engine: &mut Engine, call: &Syscall, waits: &mut u32) {
    match call {
        Syscall::Wait => {
            *waits += 1;
            // 1 回目のクリックで文字送りを畳み、2 回目で進む。
            engine.click();
            println!("[wait #{waits}] {}", engine.scene_text().replace('\n', "\n           "));
            engine.click();
        }
        Syscall::Sleep { ms } => engine.update(*ms as f32),
        _ => {}
    }
    if !engine.take_finished_call() {
        eprintln!("[biwa] {} did not finish in headless mode; continuing", call.name());
    }
}

fn trace(options: &Options, call: &Syscall) {
    if options.trace {
        println!("  {call:?}");
    }
}

fn report(stats: &VmStats, waits: u32, total: Duration) {
    println!("[biwa] the scene finished");
    println!("  waits            {waits}");
    println!("  compile          {:.1} ms", ms(stats.compile));
    println!("  wasm running     {:.1} ms", ms(stats.running));
    println!("  host calls       {}", stats.host_calls);
    println!("  strings created  {}", stats.strings_created);
    println!("  max gap          {:.3} ms (between host calls; GC pauses show up here)", ms(stats.max_gap));
    println!("  gaps over 1 ms   {}", stats.gaps_over_1ms);
    println!("  total            {:.1} ms", ms(total));
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
