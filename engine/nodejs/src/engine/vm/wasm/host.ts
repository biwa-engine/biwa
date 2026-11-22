/**
 * メインスレッド側の kernel。wasm から届いた syscall をエンジンの API (`api/*`) に流す。
 *
 * scene の中断は Worker のブロック (`Atomics.wait`) で起きる。
 */

import {
  clearContent,
  flushContent,
  pushContentText,
  waitForClick,
} from "../../api/message";
import {
  addTransition,
  awaitTransitions,
  createObject,
  deleteObject,
  sleep,
  startTransitions,
} from "../../api/object";
import {
  createUiElement,
  pushUiChild,
  setUiHandler,
  setUiValue,
  setUiProperty,
  setUiPropertyString,
} from "../../api/ui";
import {
  soundAddStart,
  soundAddStop,
  soundAddTransition,
  soundBusAddEffect,
  soundEffectCreate,
  soundBusAddStopAll,
  soundBusAddTransition,
  soundBusAttach,
  soundBusCreate,
  soundBusDelete,
  soundBusSetParam,
  soundCreate,
  soundEffectAddTransition,
  soundEffectSetParam,
  soundOutputAttach,
  soundPreload,
} from "../../api/sound";
import { engine } from "../../api/context";
import {
  completeCall,
  createChannelBuffer,
  failCall,
  type WorkerMessage,
} from "./bridge";
import { UiContractError } from "../../ui/UIObjects";
import { ENTER_SCENE_PAGE } from "./contract";
import type { ReleaseMessage, StartMessage, StartSceneMessage } from "./worker";

/**
 * syscall の実装。
 *
 * ブロッキング syscall は Promise を返してよい。解決するまで wasm は止まったままで、
 * その間もこのスレッドは描画とイベント処理を続ける。
 */
type SyscallHandler = (...args: never[]) => unknown;

/**
 * syscall 名から実装への対応表。
 *
 * どれをブロッキングにするかは `contract.ts` が決めている。
 * ここにあるのは中身だけである。
 *
 * `sys_canvas_object_create` の先頭の `id` は Worker が採ったものである
 * (`alloc`。メインスレッドと往復せずに戻り値を返すため)。
 */
function createHandlers(): Record<string, SyscallHandler> {
  return {
    sys_content_push_text: (
      uiId: number,
      text: string,
      speed: number,
      sizeUnit: number,
      sizeValue: number,
      weight: number,
      r: number,
      g: number,
      b: number,
      a: number,
    ) =>
      pushContentText(
        uiId,
        text,
        speed,
        sizeUnit,
        sizeValue,
        weight,
        r,
        g,
        b,
        a,
      ),

    sys_content_flush: (uiId: number) => flushContent(uiId),

    sys_content_clear: (uiId: number) => clearContent(uiId),

    sys_timeline_wait_click: () => waitForClick(),

    sys_canvas_object_create: (
      id: number,
      canvasId: number,
      path: string,
      layer: number,
      x: number,
      y: number,
      w: number,
      h: number,
      alpha: number,
      theta: number,
    ) => createObject(id, canvasId, path, layer, x, y, w, h, alpha, theta),

    sys_canvas_object_delete: (id: number, after: number) => deleteObject(id, after),

    sys_canvas_object_add_transition: (
      id: number,
      param: number,
      kind: number,
      valI: number,
      valF: number,
      after: number,
      duration: number,
    ) => addTransition(id, param, kind, valI, valF, after, duration),

    sys_timeline_fire: (sync: number) => startTransitions(sync),

    sys_timeline_await_synced: () => awaitTransitions(),

    sys_timeline_sleep: (ms: number) => sleep(ms),

    // 音。`alloc` のものは先頭が Worker の採った id (`ALLOC_SPACE` の空間ごと)。
    sys_sound_bus_create: (bus: number) => soundBusCreate(bus),
    sys_sound_effect_create: (effect: number, kind: number, cfg1: number, cfg2: number) =>
      soundEffectCreate(effect, kind, cfg1, cfg2),
    sys_sound_bus_add_effect: (bus: number, effect: number) => soundBusAddEffect(bus, effect),
    sys_sound_bus_attach: (bus: number, parent: number) => soundBusAttach(bus, parent),
    sys_sound_output_attach: (bus: number) => soundOutputAttach(bus),
    sys_sound_bus_add_stop_all: (bus: number, after: number) => soundBusAddStopAll(bus, after),
    sys_sound_bus_delete: (bus: number, after: number) => soundBusDelete(bus, after),
    sys_sound_bus_add_transition: (
      bus: number,
      param: number,
      kind: number,
      value: number,
      after: number,
      duration: number,
    ) => soundBusAddTransition(bus, param, kind, value, after, duration),
    sys_sound_bus_set_param: (bus: number, param: number, value: number) => soundBusSetParam(bus, param, value),
    sys_sound_effect_add_transition: (
      effect: number,
      param: number,
      kind: number,
      value: number,
      after: number,
      duration: number,
    ) => soundEffectAddTransition(effect, param, kind, value, after, duration),
    sys_sound_effect_set_param: (effect: number, param: number, value: number) =>
      soundEffectSetParam(effect, param, value),
    sys_sound_object_create: (
      id: number,
      path: string,
      bus: number,
      volumeDb: number,
      x: number,
      y: number,
      loopStartMs: number,
      loopEndMs: number,
      normalize: number,
    ) => soundCreate(id, path, bus, volumeDb, x, y, loopStartMs, loopEndMs, normalize),
    sys_sound_object_add_start: (id: number, after: number, offsetMs: number, repeat: number) =>
      soundAddStart(id, after, offsetMs, repeat),
    sys_sound_object_add_stop: (id: number, after: number) => soundAddStop(id, after),
    sys_sound_object_add_transition: (
      id: number,
      param: number,
      kind: number,
      value: number,
      after: number,
      duration: number,
    ) => soundAddTransition(id, param, kind, value, after, duration),
    sys_sound_preload: (path: string) => soundPreload(path),

    sys_ui_create: (id: number, kind: number) => createUiElement(id, kind),

    sys_ui_set_property: (
      id: number,
      kind: number,
      valU1: number,
      valU2: number,
      valU3: number,
      valU4: number,
      valI1: number,
      valI2: number,
      valF: number,
    ) =>
      setUiProperty(id, kind, valU1, valU2, valU3, valU4, valI1, valI2, valF),

    sys_ui_set_property_with_string: (
      id: number,
      kind: number,
      valU: number,
      valI: number,
      valF: number,
      valS: string,
    ) => setUiPropertyString(id, kind, valU, valI, valF, valS),

    sys_ui_push_child: (parent: number, child: number) =>
      pushUiChild(parent, child),

    // `handle` は Worker が関数を預かって振った番号 (`contract.ts` の `retain`)。
    sys_ui_set_handler: (id: number, kind: number, handle: number) =>
      setUiHandler(id, kind, handle),
    // `handle` は Worker が値を預かって振った番号 (同じく `retain`)。
    sys_ui_set_value: (id: number, kind: number, handle: number) =>
      setUiValue(id, kind, handle),

    // Worker 自身が流す cast (`contract.ts`)。scene を始めるときに ScenePage を見せる。
    [ENTER_SCENE_PAGE]: (windowId: number) => engine().ui.enterScenePage(windowId),
  };
}

/**
 * wasm の生成物を Worker で走らせ、終わるまで待つ。
 *
 * Worker はまず `fn main()` (`__biwa_entrypoint`) で Window を表示し、SceneStartButton が押されたら
 * その `on_click` と Window の `main_scene` で scene を始める。
 * 返る Promise はゲームが最後まで進んだときに解決する。
 */
export function runWasm(url: string): Promise<void> {
  // 要るのは `SharedArrayBuffer` そのものなので、`crossOriginIsolated` ではなくこちらを見る。
  // ブラウザは cross-origin isolated でないページから `SharedArrayBuffer` を隠すので、
  // ふつうは同じ判定になる。違うのはデスクトップ版の WebView で、
  // Linux (WebKitGTK) ではプレイヤーの独自スキーム (`biwa://`) が isolated と見なされないまま、
  // プレイヤーが有効にした `SharedArrayBuffer` は使える (`engine/desktop-tauri`)。
  if (typeof SharedArrayBuffer === "undefined") {
    return Promise.reject(
      new Error(
        "[biwa] SharedArrayBuffer is unavailable (the page is not cross-origin isolated), " +
        "so the wasm runtime cannot block on syscalls. " +
        "The server must send `Cross-Origin-Opener-Policy: same-origin` and " +
        "`Cross-Origin-Embedder-Policy: require-corp`.",
      ),
    );
  }

  const handlers = createHandlers();
  const buffer = createChannelBuffer();
  const worker = new Worker(new URL("./worker.ts", import.meta.url), {
    type: "module",
    name: "biwa-vm",
  });

  return new Promise<void>((resolve, reject) => {
    const finish = (done: () => void): void => {
      engine().ui.setRetainedReleaser(() => {});
      engine().ui.setSceneStarter(() => {});
      worker.terminate();
      done();
    };

    // Element が消えたら、預けた関数と値を Worker に手放させる。
    engine().ui.setRetainedReleaser((handles) => {
      const release: ReleaseMessage = { kind: "release", handles };
      worker.postMessage(release);
    });

    worker.addEventListener("message", (event: MessageEvent<WorkerMessage>) => {
      const message = event.data;

      switch (message.kind) {
        case "syscalls":
          // 止まらない syscall のまとまり。1 通で来るので、
          // この間にフレームが挟まることはない。
          for (const call of message.calls) {
            const fatal = dispatch(handlers, call.name, call.args);
            if (fatal !== null) {
              finish(() => reject(fatal));
              return;
            }
          }
          return;
        case "syscall":
          void serve(buffer, handlers, message.name, message.args);
          return;
        case "ready":
          return;
        case "exit":
          finish(resolve);
          return;
        case "error":
          finish(() => reject(new Error(message.message)));
          return;
      }
    });

    // Worker 自体が壊れた場合 (読み込み失敗など)。
    worker.addEventListener("error", (event: ErrorEvent) => {
      finish(() =>
        reject(new Error(`[biwa] the wasm worker failed: ${event.message}`)),
      );
    });

    const start: StartMessage = { kind: "start", url, buffer };
    worker.postMessage(start);

    // SceneStartButton が押されたら、Worker に scene を始めさせる。
    // 2 回目以降の開始は未定義 (§19 の R8) なので、最初の 1 回だけ送る。
    let sceneStarted = false;
    engine().ui.setSceneStarter((request) => {
      if (sceneStarted) return;
      sceneStarted = true;
      const message: StartSceneMessage = { kind: "startScene", ...request };
      worker.postMessage(message);
    });
  });
}

/**
 * 止まらない syscall を 1 つ処理する。
 *
 * 呼び出し元は既に返っているので、失敗しても伝える先が無い。ログに出す。
 * ただし続けてもゲームとして成り立たない誤り (`UiContractError`) は返し、
 * 呼んだ側が実行を失敗させる。
 */
function dispatch(
  handlers: Record<string, SyscallHandler>,
  name: string,
  args: unknown[],
): Error | null {
  const handler = handlers[name];
  if (handler === undefined) {
    console.error(`[biwa] unknown syscall: ${name}`);
    return null;
  }

  try {
    (handler as (...a: unknown[]) => unknown)(...args);
  } catch (e) {
    if (e instanceof UiContractError) {
      return e;
    }
    console.error(`[biwa] syscall \`${name}\` failed:`, e);
  }
  return null;
}

/**
 * ブロッキング syscall を 1 つ処理する。
 *
 * 結果を共有バッファに書いて Worker を起こすところまでが仕事である。
 * 起こし忘れると wasm は永久に止まるので、失敗も必ず書き戻す。
 */
async function serve(
  buffer: SharedArrayBuffer,
  handlers: Record<string, SyscallHandler>,
  name: string,
  args: unknown[],
): Promise<void> {
  const handler = handlers[name];

  if (handler === undefined) {
    failCall(buffer, `[biwa] unknown syscall: ${name}`);
    return;
  }

  try {
    const result = await (handler as (...a: unknown[]) => unknown)(...args);
    completeCall(buffer, result);
  } catch (e) {
    failCall(buffer, e instanceof Error ? e.message : String(e));
  }
}
