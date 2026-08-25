import type { BiwaSyscall } from "./syscall";

/**
 * scene の実行体。コンパイラは scene を generator function として出力する。
 *
 * - `next(v)` で再開する。`v` が直前の syscall の戻り値になる
 * - `yield` された値が syscall (VM exit)
 */
export type BiwaScene = Generator<unknown, unknown, unknown>;

/**
 * syscall の実装。
 *
 * Promise を返せばブロッキング syscall で、解決するまで scene は再開しない。
 * 値をそのまま返せば非ブロッキング syscall で、scene はそのまま走り続ける。
 */
export type SyscallHandler = (...args: never[]) => unknown;

export type SyscallTable = Record<number, SyscallHandler>;

/** 非ブロッキング syscall が続いたとき、これだけ経ったら一度描画に譲る。 */
const FRAME_BUDGET_MS = 8;

/**
 * scene を駆動する。ゲームコードから見れば kernel land にあたる。
 *
 * scene が `yield` するたびに制御がここへ移り、syscall を処理して、
 * 結果を `next()` で書き戻して再開する。
 * scene を再開しない限り止まったままなので、
 * ポーズ・スキップ・速度調整はこのループの外側で決められる。
 */
export class Kernel {
  private readonly table: SyscallTable;

  constructor(table: SyscallTable) {
    this.table = table;
  }

  async run(scene: BiwaScene): Promise<unknown> {
    let send: unknown;
    let lastYieldedAt = performance.now();

    for (;;) {
      const { value, done } = scene.next(send);
      if (done) return value;

      const call = value as BiwaSyscall;
      const handler = this.table[call.sys];
      if (handler === undefined) {
        throw new Error(`[biwa] unknown syscall: ${JSON.stringify(value)}`);
      }

      const result = (handler as (...args: unknown[]) => unknown)(...call.args);

      if (isPromise(result)) {
        // ブロッキング syscall。完了するまで scene は止まったままになる。
        send = await result;
        lastYieldedAt = performance.now();
        continue;
      }

      send = result;

      // 非ブロッキング syscall が続いても描画が止まらないようにする。
      if (performance.now() - lastYieldedAt > FRAME_BUDGET_MS) {
        await nextFrame();
        lastYieldedAt = performance.now();
      }
    }
  }
}

function isPromise(value: unknown): value is Promise<unknown> {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as { then?: unknown }).then === "function"
  );
}

function nextFrame(): Promise<void> {
  return new Promise((resolve) => {
    requestAnimationFrame(() => resolve());
  });
}
