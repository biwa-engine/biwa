//! 生成物の import (`biwa:engine` / `biwa:runtime`) の実装。
//!
//! 取り決めは `engine/nodejs/src/engine/vm/wasm/contract.ts` と同じで、
//! 型 (引数の並び) は std の `.wat` の import 宣言が正である。
//!
//! 区分は JS 版より少ない。止まるか (`call`) 止まらないか (`cast`) だけで、
//! wasm の値を触るだけのもの (JS 版の `local`) はこのスレッドで済ませて返す。
//! JS 版の `alloc` (id をこちらで採番して止まらずに返す) と
//! `retain` (関数・値を表に預けて番号だけ送る) は形を変えずに残している。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use biwa_player_core::syscall::Syscall;
use wasmtime::{AnyRef, Caller, ExternRef, Func, Linker, OwnedRooted, Result, Rooted};

use crate::values::{self, HostMap, HostVec};
use crate::{VmEvent, VmStats, reply_channel};

/// std が書くエンジン API の import 名前空間。
const ENGINE: &str = "biwa:engine";
/// コンパイラ自身が要求する import の名前空間 (文字列リテラルの実体化)。
const RUNTIME: &str = "biwa:runtime";

/// まとめておける止まらない syscall の数。超えたら送り出す (走り続けるシーンへの保険)。
const BATCH_LIMIT: usize = 64;

/// wasm のスレッドの状態 (`Store` のデータ)。
pub(crate) struct HostState {
    events: Sender<VmEvent>,
    notify: Arc<dyn Fn() + Send + Sync>,
    batch: Vec<Syscall>,
    /// UI Element と canvas オブジェクトの id の連番 (JS 版の `alloc`)。
    next_id: u32,
    /// 音の id の連番。SoundObject・Bus・Effect で空間を分ける (`docs/sound.md`)。
    next_sound: u32,
    next_bus: u32,
    next_effect: u32,
    /// 預かった関数 (JS 版の `retain`)。ホストが後で呼ぶ。
    pub(crate) handlers: HashMap<u32, Func>,
    /// 預かった値 (JS 版の `retain`)。ホストが後で host export の引数として返す。
    /// GC の対象なので [`OwnedRooted`] で握る (握っている間は回収されない)。
    pub(crate) values: HashMap<u32, OwnedRooted<AnyRef>>,
    /// 関数と値で共有する番号の連番。0 は「無し」。
    next_handle: u32,
    pub(crate) stats: VmStats,
    /// 最後にホスト関数から wasm に戻った時刻。
    last_return: Instant,
}

impl HostState {
    pub(crate) fn new(events: Sender<VmEvent>, notify: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            events,
            notify,
            batch: Vec::new(),
            next_id: 1,
            next_sound: 1,
            next_bus: 1,
            next_effect: 1,
            handlers: HashMap::new(),
            values: HashMap::new(),
            next_handle: 1,
            stats: VmStats::default(),
            last_return: Instant::now(),
        }
    }

    /// エンジンへ知らせる。
    pub(crate) fn send(&mut self, event: VmEvent) {
        let _ = self.events.send(event);
        (self.notify)();
    }

    /// 止まらない syscall を積む。
    pub(crate) fn cast(&mut self, syscall: Syscall) {
        let flush = syscall.flushes_batch();
        self.batch.push(syscall);
        if flush || self.batch.len() >= BATCH_LIMIT {
            self.flush();
        }
    }

    /// 積んである syscall を送り出す。
    pub(crate) fn flush(&mut self) {
        if !self.batch.is_empty() {
            let batch = std::mem::take(&mut self.batch);
            self.send(VmEvent::Batch(batch));
        }
    }

    /// 止まる syscall。エンジンの返事が来るまでこのスレッドを止める。
    fn call(&mut self, syscall: Syscall) -> Result<()> {
        // 先に積んだものを追い越さないよう、必ず流してから止まる。
        self.flush();
        let name = syscall.name();
        let (reply, rx) = reply_channel();
        self.send(VmEvent::Call(syscall, reply));
        match rx.recv() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(wasmtime::format_err!("syscall `{name}` failed: {message}")),
            Err(_) => Err(wasmtime::format_err!("the engine went away while `{name}` was waiting")),
        }
    }

    fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn alloc_in(counter: &mut u32) -> u32 {
        let id = *counter;
        *counter += 1;
        id
    }

    fn next_handle(&mut self) -> u32 {
        let handle = self.next_handle;
        self.next_handle += 1;
        handle
    }

    fn retain(&mut self, func: Func) -> u32 {
        let handle = self.next_handle();
        self.handlers.insert(handle, func);
        handle
    }

    fn retain_value(&mut self, value: OwnedRooted<AnyRef>) -> u32 {
        let handle = self.next_handle();
        self.values.insert(handle, value);
        handle
    }

    /// 預けたものを手放す。関数か値かは問わない (番号の空間は共有)。知らない番号は無視する。
    pub(crate) fn release(&mut self, handles: impl IntoIterator<Item = u32>) {
        for h in handles {
            self.handlers.remove(&h);
            self.values.remove(&h);
        }
    }

    /// ホスト関数に入った。前に wasm へ戻ってからの時間を数える。
    fn enter(&mut self) {
        let gap = self.last_return.elapsed();
        let s = &mut self.stats;
        s.host_calls += 1;
        s.running += gap;
        s.max_gap = s.max_gap.max(gap);
        if gap > Duration::from_millis(1) {
            s.gaps_over_1ms += 1;
        }
    }

    /// ホスト関数から wasm へ戻る。
    fn leave(&mut self) {
        self.last_return = Instant::now();
    }

    /// wasm を呼び始める直前 (ホストから wasm へ入る)。
    pub(crate) fn start_running(&mut self) {
        self.last_return = Instant::now();
    }

    /// wasm から戻った直後。最後の区間を数える。
    pub(crate) fn stop_running(&mut self) {
        self.stats.running += self.last_return.elapsed();
    }
}

/// ホスト関数の入口と出口で統計を取る。
fn traced<R>(caller: &mut Caller<'_, HostState>, f: impl FnOnce(&mut Caller<'_, HostState>) -> R) -> R {
    caller.data_mut().enter();
    let r = f(caller);
    caller.data_mut().leave();
    r
}

pub(crate) fn define(linker: &mut Linker<HostState>) -> Result<()> {
    define_runtime(linker)?;
    define_content(linker)?;
    define_canvas(linker)?;
    define_values(linker)?;
    define_ui(linker)?;
    define_sound(linker)?;
    Ok(())
}

/// 音 (`docs/sound.md`)。止まる syscall は無い。id は空間ごとにこちらで採番して返す (`alloc`)。
fn define_sound(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(ENGINE, "sys_sound_bus_create", |mut caller: Caller<'_, HostState>| -> i32 {
        traced(&mut caller, |c| {
            let state = c.data_mut();
            let bus = HostState::alloc_in(&mut state.next_bus);
            state.cast(Syscall::SoundBusCreate { bus });
            bus as i32
        })
    })?;
    linker.func_wrap(
        ENGINE,
        "sys_sound_effect_create",
        |mut caller: Caller<'_, HostState>, kind: i32, cfg1: i32, cfg2: i32| -> i32 {
            traced(&mut caller, |c| {
                let state = c.data_mut();
                let effect = HostState::alloc_in(&mut state.next_effect);
                state.cast(Syscall::SoundEffectCreate { effect, kind, cfg1, cfg2 });
                effect as i32
            })
        },
    )?;
    linker.func_wrap(ENGINE, "sys_sound_bus_add_effect", |mut caller: Caller<'_, HostState>, bus: i32, effect: i32| {
        traced(&mut caller, |c| c.data_mut().cast(Syscall::SoundBusAddEffect { bus: bus as u32, effect: effect as u32 }));
    })?;
    linker.func_wrap(ENGINE, "sys_sound_bus_attach", |mut caller: Caller<'_, HostState>, bus: i32, parent: i32| {
        traced(&mut caller, |c| c.data_mut().cast(Syscall::SoundBusAttach { bus: bus as u32, parent: parent as u32 }));
    })?;
    linker.func_wrap(ENGINE, "sys_sound_output_attach", |mut caller: Caller<'_, HostState>, bus: i32| {
        traced(&mut caller, |c| c.data_mut().cast(Syscall::SoundOutputAttach { bus: bus as u32 }));
    })?;
    linker.func_wrap(ENGINE, "sys_sound_bus_add_stop_all", |mut caller: Caller<'_, HostState>, bus: i32, after: i32| {
        traced(&mut caller, |c| c.data_mut().cast(Syscall::SoundBusAddStopAll { bus: bus as u32, after }));
    })?;
    linker.func_wrap(ENGINE, "sys_sound_bus_delete", |mut caller: Caller<'_, HostState>, bus: i32, after: i32| {
        traced(&mut caller, |c| c.data_mut().cast(Syscall::SoundBusDelete { bus: bus as u32, after }));
    })?;
    linker.func_wrap(
        ENGINE,
        "sys_sound_bus_add_transition",
        |mut caller: Caller<'_, HostState>, bus: i32, param: i32, kind: i32, value: f32, after: i32, duration: i32| {
            traced(&mut caller, |c| {
                c.data_mut().cast(Syscall::SoundBusAddTransition { bus: bus as u32, param, kind, value, after, duration })
            });
        },
    )?;
    linker.func_wrap(
        ENGINE,
        "sys_sound_bus_set_param",
        |mut caller: Caller<'_, HostState>, bus: i32, param: i32, value: f32| {
            traced(&mut caller, |c| c.data_mut().cast(Syscall::SoundBusSetParam { bus: bus as u32, param, value }));
        },
    )?;
    linker.func_wrap(
        ENGINE,
        "sys_sound_effect_add_transition",
        |mut caller: Caller<'_, HostState>, effect: i32, param: i32, kind: i32, value: f32, after: i32, duration: i32| {
            traced(&mut caller, |c| {
                c.data_mut().cast(Syscall::SoundEffectAddTransition {
                    effect: effect as u32,
                    param,
                    kind,
                    value,
                    after,
                    duration,
                })
            });
        },
    )?;
    linker.func_wrap(
        ENGINE,
        "sys_sound_effect_set_param",
        |mut caller: Caller<'_, HostState>, effect: i32, param: i32, value: f32| {
            traced(&mut caller, |c| {
                c.data_mut().cast(Syscall::SoundEffectSetParam { effect: effect as u32, param, value })
            });
        },
    )?;
    linker.func_wrap(
        ENGINE,
        "sys_sound_create",
        |mut caller: Caller<'_, HostState>,
         path: Option<Rooted<ExternRef>>,
         bus: i32,
         volume_db: f32,
         x: f32,
         y: f32,
         loop_start_ms: i32,
         loop_end_ms: i32,
         normalize: i32|
         -> Result<i32> {
            traced(&mut caller, |caller| {
                let path = values::string_of(&*caller, path)?;
                let state = caller.data_mut();
                let id = HostState::alloc_in(&mut state.next_sound);
                state.cast(Syscall::SoundCreate {
                    id,
                    path,
                    bus: bus as u32,
                    volume_db,
                    x,
                    y,
                    loop_start_ms,
                    loop_end_ms,
                    normalize,
                });
                Ok(id as i32)
            })
        },
    )?;
    linker.func_wrap(
        ENGINE,
        "sys_sound_add_start",
        |mut caller: Caller<'_, HostState>, id: i32, after: i32, offset_ms: i32, repeat: i32| {
            traced(&mut caller, |c| {
                c.data_mut().cast(Syscall::SoundAddStart { id: id as u32, after, offset_ms, repeat })
            });
        },
    )?;
    linker.func_wrap(ENGINE, "sys_sound_add_stop", |mut caller: Caller<'_, HostState>, id: i32, after: i32| {
        traced(&mut caller, |c| c.data_mut().cast(Syscall::SoundAddStop { id: id as u32, after }));
    })?;
    linker.func_wrap(
        ENGINE,
        "sys_sound_add_transition",
        |mut caller: Caller<'_, HostState>, id: i32, param: i32, kind: i32, value: f32, after: i32, duration: i32| {
            traced(&mut caller, |c| {
                c.data_mut().cast(Syscall::SoundAddTransition { id: id as u32, param, kind, value, after, duration })
            });
        },
    )?;
    linker.func_wrap(
        ENGINE,
        "sys_sound_preload",
        |mut caller: Caller<'_, HostState>, path: Option<Rooted<ExternRef>>| -> Result<()> {
            traced(&mut caller, |caller| {
                let path = values::string_of(&*caller, path)?;
                caller.data_mut().cast(Syscall::SoundPreload { path });
                Ok(())
            })
        },
    )?;
    Ok(())
}

fn define_runtime(linker: &mut Linker<HostState>) -> Result<()> {
    // 文字列リテラルの実体化。データセグメント上の UTF-8 を文字列にする。
    linker.func_wrap(
        RUNTIME,
        "string_const",
        |mut caller: Caller<'_, HostState>, offset: i32, len: i32| -> Result<Option<Rooted<ExternRef>>> {
            traced(&mut caller, |caller| {
                let memory = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .ok_or_else(|| wasmtime::format_err!("the generated module does not export its memory"))?;
                let (start, len) = (offset as u32 as usize, len as u32 as usize);
                let bytes = memory
                    .data(&*caller)
                    .get(start..start + len)
                    .ok_or_else(|| wasmtime::format_err!("string literal out of bounds"))?;
                let text = String::from_utf8_lossy(bytes).into_owned();
                caller.data_mut().stats.strings_created += 1;
                values::new_string(caller, text).map(Some)
            })
        },
    )?;
    Ok(())
}

fn define_content(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        ENGINE,
        "sys_content_push_text",
        |mut caller: Caller<'_, HostState>,
         ui: i32,
         text: Option<Rooted<ExternRef>>,
         speed: f32,
         size_unit: i32,
         size_value: f32,
         weight: i32,
         r: i32,
         g: i32,
         b: i32,
         a: i32|
         -> Result<()> {
            traced(&mut caller, |caller| {
                let text = values::string_of(&*caller, text)?;
                caller.data_mut().cast(Syscall::ContentPushText {
                    ui: ui as u32,
                    text,
                    speed,
                    size_unit,
                    size_value,
                    weight,
                    r,
                    g,
                    b,
                    a,
                });
                Ok(())
            })
        },
    )?;
    linker.func_wrap(ENGINE, "sys_content_flush", |mut caller: Caller<'_, HostState>, ui: i32| {
        traced(&mut caller, |c| c.data_mut().cast(Syscall::ContentFlush { ui: ui as u32 }));
    })?;
    linker.func_wrap(ENGINE, "sys_content_clear", |mut caller: Caller<'_, HostState>, ui: i32| {
        traced(&mut caller, |c| c.data_mut().cast(Syscall::ContentClear { ui: ui as u32 }));
    })?;
    linker.func_wrap(ENGINE, "sys_wait", |mut caller: Caller<'_, HostState>| -> Result<()> {
        traced(&mut caller, |c| c.data_mut().call(Syscall::Wait))
    })?;
    Ok(())
}

fn define_canvas(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        ENGINE,
        "sys_create_object",
        |mut caller: Caller<'_, HostState>,
         canvas: i32,
         path: Option<Rooted<ExternRef>>,
         layer: i32,
         x: f32,
         y: f32,
         width: f32,
         height: f32,
         alpha: i32,
         theta: f32|
         -> Result<i32> {
            traced(&mut caller, |caller| {
                let path = values::string_of(&*caller, path)?;
                let state = caller.data_mut();
                let id = state.alloc_id();
                state.cast(Syscall::CreateObject {
                    id,
                    canvas: canvas as u32,
                    path,
                    layer,
                    x,
                    y,
                    width,
                    height,
                    alpha,
                    theta,
                });
                Ok(id as i32)
            })
        },
    )?;
    linker.func_wrap(ENGINE, "sys_delete_object", |mut caller: Caller<'_, HostState>, id: i32, after: i32| {
        traced(&mut caller, |c| c.data_mut().cast(Syscall::DeleteObject { id: id as u32, after }));
    })?;
    linker.func_wrap(
        ENGINE,
        "sys_add_transition",
        |mut caller: Caller<'_, HostState>,
         id: i32,
         param: i32,
         kind: i32,
         val_i: i32,
         val_f: f32,
         after: i32,
         duration: i32| {
            traced(&mut caller, |c| {
                c.data_mut().cast(Syscall::AddTransition {
                    id: id as u32,
                    param,
                    kind,
                    val_i,
                    val_f,
                    after,
                    duration,
                });
            });
        },
    )?;
    linker.func_wrap(ENGINE, "sys_start_transitions", |mut caller: Caller<'_, HostState>, sync: i32| {
        traced(&mut caller, |c| c.data_mut().cast(Syscall::StartTransitions { sync }));
    })?;
    linker.func_wrap(ENGINE, "sys_await_transitions", |mut caller: Caller<'_, HostState>| -> Result<()> {
        traced(&mut caller, |c| c.data_mut().call(Syscall::AwaitTransitions))
    })?;
    linker.func_wrap(ENGINE, "sys_sleep", |mut caller: Caller<'_, HostState>, ms: i32| -> Result<()> {
        traced(&mut caller, |c| c.data_mut().call(Syscall::Sleep { ms }))
    })?;
    Ok(())
}

/// wasm の値に対する操作。エンジンには行かない。
fn define_values(linker: &mut Linker<HostState>) -> Result<()> {
    type Ext = Option<Rooted<ExternRef>>;
    type Any = Option<Rooted<AnyRef>>;

    linker.func_wrap(
        ENGINE,
        "sys_string_eq",
        |mut caller: Caller<'_, HostState>, a: Ext, b: Ext| -> Result<i32> {
            traced(&mut caller, |caller| {
                Ok(i32::from(values::string_of(&*caller, a)? == values::string_of(&*caller, b)?))
            })
        },
    )?;
    linker.func_wrap(
        ENGINE,
        "sys_string_concat",
        |mut caller: Caller<'_, HostState>, a: Ext, b: Ext| -> Result<Ext> {
            traced(&mut caller, |caller| {
                let text = values::string_of(&*caller, a)? + &values::string_of(&*caller, b)?;
                caller.data_mut().stats.strings_created += 1;
                values::new_string(caller, text).map(Some)
            })
        },
    )?;
    linker.func_wrap(
        ENGINE,
        "sys_int_to_string",
        |mut caller: Caller<'_, HostState>, value: i32| -> Result<Ext> {
            traced(&mut caller, |caller| {
                caller.data_mut().stats.strings_created += 1;
                values::new_string(caller, value.to_string()).map(Some)
            })
        },
    )?;
    linker.func_wrap(
        ENGINE,
        "sys_float_to_string",
        |mut caller: Caller<'_, HostState>, value: f32| -> Result<Ext> {
            traced(&mut caller, |caller| {
                caller.data_mut().stats.strings_created += 1;
                values::new_string(caller, values::float_to_string(value)).map(Some)
            })
        },
    )?;

    linker.func_wrap(ENGINE, "sys_vec_new", |mut caller: Caller<'_, HostState>| -> Result<Ext> {
        traced(&mut caller, |caller| ExternRef::new(caller, HostVec(Vec::new())).map(Some))
    })?;
    linker.func_wrap(
        ENGINE,
        "sys_vec_of",
        |mut caller: Caller<'_, HostState>, value: Any| -> Result<Ext> {
            traced(&mut caller, |caller| {
                let value = values::own(&mut *caller, value)?;
                ExternRef::new(caller, HostVec(vec![value])).map(Some)
            })
        },
    )?;
    linker.func_wrap(
        ENGINE,
        "sys_vec_push",
        |mut caller: Caller<'_, HostState>, vec: Ext, value: Any| -> Result<()> {
            traced(&mut caller, |caller| {
                let value = values::own(&mut *caller, value)?;
                with_mut::<HostVec, _>(caller, vec, |v| v.0.push(value))
            })
        },
    )?;
    linker.func_wrap(
        ENGINE,
        "sys_vec_len",
        |mut caller: Caller<'_, HostState>, vec: Ext| -> Result<i32> {
            traced(&mut caller, |caller| with_mut::<HostVec, _>(caller, vec, |v| v.0.len() as i32))
        },
    )?;
    linker.func_wrap(
        ENGINE,
        "sys_vec_get",
        |mut caller: Caller<'_, HostState>, vec: Ext, index: i32| -> Result<Any> {
            traced(&mut caller, |caller| {
                // 範囲外は Option::none (null)。
                let item = with_mut::<HostVec, _>(caller, vec, |v| {
                    usize::try_from(index).ok().and_then(|i| v.0.get(i).cloned()).flatten()
                })?;
                Ok(item.map(|owned| owned.to_rooted(caller)))
            })
        },
    )?;

    linker.func_wrap(
        ENGINE,
        "sys_map_insert",
        |mut caller: Caller<'_, HostState>, map: Ext, key: Any, value: Any| -> Result<()> {
            traced(&mut caller, |caller| {
                let index = map_find(caller, map.clone(), key.clone())?;
                let value = values::own(&mut *caller, value)?;
                let key = values::own(&mut *caller, key)?;
                with_mut::<HostMap, _>(caller, map, |m| match index {
                    Some(i) => m.0[i].1 = value,
                    None => m.0.push((key, value)),
                })
            })
        },
    )?;
    linker.func_wrap(
        ENGINE,
        "sys_map_get",
        |mut caller: Caller<'_, HostState>, map: Ext, key: Any| -> Result<Any> {
            traced(&mut caller, |caller| {
                // 未登録は Option::none (null)。
                let Some(index) = map_find(caller, map.clone(), key)? else {
                    return Ok(None);
                };
                let value = with_mut::<HostMap, _>(caller, map, |m| m.0[index].1.clone())?;
                Ok(value.map(|owned| owned.to_rooted(caller)))
            })
        },
    )?;
    Ok(())
}

fn define_ui(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(ENGINE, "sys_ui_create", |mut caller: Caller<'_, HostState>, kind: i32| -> i32 {
        traced(&mut caller, |c| {
            let state = c.data_mut();
            let id = state.alloc_id();
            state.cast(Syscall::UiCreate { id, kind });
            id as i32
        })
    })?;
    linker.func_wrap(
        ENGINE,
        "sys_ui_set_property",
        |mut caller: Caller<'_, HostState>,
         id: i32,
         kind: i32,
         u1: i32,
         u2: i32,
         u3: i32,
         u4: i32,
         i1: i32,
         i2: i32,
         f: f32| {
            traced(&mut caller, |c| {
                c.data_mut().cast(Syscall::UiSetProperty { id: id as u32, kind, u1, u2, u3, u4, i1, i2, f });
            });
        },
    )?;
    linker.func_wrap(
        ENGINE,
        "sys_ui_set_property_with_string",
        |mut caller: Caller<'_, HostState>,
         id: i32,
         kind: i32,
         u: i32,
         i: i32,
         f: f32,
         s: Option<Rooted<ExternRef>>|
         -> Result<()> {
            traced(&mut caller, |caller| {
                let s = values::string_of(&*caller, s)?;
                caller
                    .data_mut()
                    .cast(Syscall::UiSetPropertyWithString { id: id as u32, kind, u, i, f, s });
                Ok(())
            })
        },
    )?;
    linker.func_wrap(ENGINE, "sys_ui_push_child", |mut caller: Caller<'_, HostState>, parent: i32, child: i32| {
        traced(&mut caller, |c| {
            c.data_mut().cast(Syscall::UiPushChild { parent: parent as u32, child: child as u32 });
        });
    })?;
    // 関数は表に預けて番号に替える。関数そのものはこのスレッドに残り、ホストが後で呼ぶ
    // (`docs/host-function-values.md`)。
    linker.func_wrap(
        ENGINE,
        "sys_ui_set_handler",
        |mut caller: Caller<'_, HostState>, id: i32, kind: i32, func: Option<Func>| -> Result<()> {
            traced(&mut caller, |c| {
                let func = func.ok_or_else(|| wasmtime::format_err!("sys_ui_set_handler got a null function"))?;
                let state = c.data_mut();
                let handle = state.retain(func);
                state.cast(Syscall::UiSetHandler { id: id as u32, kind, handle });
                Ok(())
            })
        },
    )?;
    linker.func_wrap(
        ENGINE,
        "sys_ui_set_value",
        |mut caller: Caller<'_, HostState>, id: i32, kind: i32, value: Option<Rooted<AnyRef>>| -> Result<()> {
            traced(&mut caller, |c| {
                let value = value.ok_or_else(|| wasmtime::format_err!("sys_ui_set_value got a null value"))?;
                let value = value.to_owned_rooted(&mut *c)?;
                let state = c.data_mut();
                let handle = state.retain_value(value);
                state.cast(Syscall::UiSetValue { id: id as u32, kind, handle });
                Ok(())
            })
        },
    )?;
    Ok(())
}

/// externref の中身 (`T`) を書き換える。
fn with_mut<T: 'static, R>(
    caller: &mut Caller<'_, HostState>,
    value: Option<Rooted<ExternRef>>,
    f: impl FnOnce(&mut T) -> R,
) -> Result<R> {
    let value = value.ok_or_else(|| wasmtime::format_err!("expected a {}, got null", short_name::<T>()))?;
    let data = value
        .data_mut(&mut *caller)?
        .and_then(|d| d.downcast_mut::<T>())
        .ok_or_else(|| wasmtime::format_err!("expected a {}", short_name::<T>()))?;
    Ok(f(data))
}

/// Map から `key` の位置を探す。
fn map_find(
    caller: &mut Caller<'_, HostState>,
    map: Option<Rooted<ExternRef>>,
    key: Option<Rooted<AnyRef>>,
) -> Result<Option<usize>> {
    let keys = with_mut::<HostMap, _>(caller, map, |m| m.0.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>())?;
    for (i, k) in keys.into_iter().enumerate() {
        let k = k.map(|owned| owned.to_rooted(&mut *caller));
        if values::same_key(&mut *caller, k, key.clone())? {
            return Ok(Some(i));
        }
    }
    Ok(None)
}

fn short_name<T>() -> &'static str {
    let name = std::any::type_name::<T>();
    name.rsplit("::").next().unwrap_or(name)
}
