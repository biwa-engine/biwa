/**
 * メインスレッド側の kernel。wasm から届いた syscall をエンジンの API に流す。
 *
 * TypeScript 経路の `vm/kernel.ts` と役割は同じで、違うのは輸送路だけである。
 * scene の中断は generator の `yield` ではなく Worker のブロックで起きる。
 * syscall の実装そのもの (`api/*`) は両者で共有している。
 */

import { createImage } from "../../api/image";
import { waitForClick, writeMessage } from "../../api/message";
import {
  completeCall,
  createChannelBuffer,
  failCall,
  type WorkerMessage,
} from "./bridge";

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
 */
function createHandlers(): Record<string, SyscallHandler> {
  return {
    sys_write: (text: string) => writeMessage(text),

    sys_wait: () => waitForClick(),

    sys_create_image: (path: string, x: number, y: number) =>
      createImage(path, x, y, {
        dx: () => 0,
        dy: () => 0,
      }),
  };
}

/**
 * wasm の生成物を Worker で走らせ、終わるまで待つ。
 *
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
      worker.terminate();
      done();
    };

    worker.addEventListener("message", (event: MessageEvent<WorkerMessage>) => {
      const message = event.data;

      switch (message.kind) {
        case "syscall":
          void serve(
            buffer,
            handlers,
            message.name,
            message.args,
            message.blocking,
          );
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

    worker.postMessage({ kind: "start", url, buffer });
  });
}

/**
 * syscall を 1 つ処理する。
 *
 * ブロッキングなら、結果を共有バッファに書いて Worker を起こすところまでが仕事である。
 * 起こし忘れると wasm は永久に止まるので、失敗も必ず書き戻す。
 */
async function serve(
  buffer: SharedArrayBuffer,
  handlers: Record<string, SyscallHandler>,
  name: string,
  args: unknown[],
  blocking: boolean,
): Promise<void> {
  const handler = handlers[name];

  if (handler === undefined) {
    const message = `[biwa] unknown syscall: ${name}`;
    if (blocking) {
      failCall(buffer, message);
    } else {
      console.error(message);
    }
    return;
  }

  try {
    const result = await (handler as (...a: unknown[]) => unknown)(...args);
    if (blocking) {
      completeCall(buffer, result);
    }
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    if (blocking) {
      failCall(buffer, message);
    } else {
      console.error(`[biwa] syscall \`${name}\` failed:`, e);
    }
  }
}
