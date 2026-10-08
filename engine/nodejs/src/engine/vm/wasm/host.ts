/**
 * メインスレッド側の kernel。wasm から届いた syscall をエンジンの API に流す。
 *
 * TypeScript 経路の `vm/kernel.ts` と役割は同じで、違うのは輸送路だけである。
 * scene の中断は generator の `yield` ではなく Worker のブロックで起きる。
 * syscall の実装そのもの (`api/*`) は両者で共有している。
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
  setUiProperty,
  setUiPropertyString,
} from "../../api/ui";
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
 * `sys_create_object` の先頭の `id` は Worker が採ったものである
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

    sys_wait: () => waitForClick(),

    sys_create_object: (
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

    sys_delete_object: (id: number, after: number) => deleteObject(id, after),

    sys_add_transition: (
      id: number,
      param: number,
      kind: number,
      valI: number,
      valF: number,
      after: number,
      duration: number,
    ) => addTransition(id, param, kind, valI, valF, after, duration),

    sys_start_transitions: (sync: number) => startTransitions(sync),

    sys_await_transitions: () => awaitTransitions(),

    sys_sleep: (ms: number) => sleep(ms),

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

    // Worker 自身が流す cast (`contract.ts`)。scene を始めるときに ScenePage を見せる。
    [ENTER_SCENE_PAGE]: (windowId: number) => engine().ui.enterScenePage(windowId),
  };
}

/**
 * wasm の生成物を Worker で走らせ、終わるまで待つ。
 *
 * Worker はまず `app()` で Window を表示し、SceneStartButton が押されたら
 * その `on_click` と Window の `main_scene` で scene を始める。
 * 返る Promise はゲームが最後まで進んだときに解決する。
 */
export function runWasm(url: string): Promise<void> {
  if (
    typeof SharedArrayBuffer === "undefined" ||
    !globalThis.crossOriginIsolated
  ) {
    return Promise.reject(
      new Error(
        "[biwa] this page is not cross-origin isolated, so the wasm runtime cannot block on syscalls. " +
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
      engine().ui.setHandlerReleaser(() => {});
      engine().ui.setSceneStarter(() => {});
      worker.terminate();
      done();
    };

    // Element が消えたら、預けた関数を Worker に手放させる。
    engine().ui.setHandlerReleaser((handles) => {
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
