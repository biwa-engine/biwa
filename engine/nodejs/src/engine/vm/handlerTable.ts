/**
 * ホストが預かる Biwa の関数 (ハンドラ) の表。
 *
 * Biwa から syscall の引数で渡された関数を番号 (handle) と引き換えに預かる。
 * 規定は `docs/host-function-values.md` にある。
 *
 * - wasm: Worker が持つ。関数 (wasm の関数のラッパー) は `postMessage` できず、
 *   UI の DOM があるメインスレッドには番号だけを送る。
 * - TypeScript: メインスレッドが持つ (`api/handler.ts`)。関数は普通の JS の関数である。
 *
 * 表が関数を握っている間はその関数は GC されない。項目は持ち主の Element が
 * 消えるときに手放す (`UIObjects`)。
 */

/**
 * 預かる関数。
 *
 * 引数と戻り値の中身はホストからは見えない (wasm の GC 参照・JS の値)。
 * どの形で呼ぶかはハンドラの種類 (`api/ui.ts` の `HandlerKind`) ごとに決まっている。
 */
export type Handler = (...args: unknown[]) => unknown;

export class HandlerTable {
  private readonly entries = new Map<number, Handler>();
  /**
   * 次に振る番号。0 は「無し」に取っておく。
   *
   * 単調増加なので決定的で、ui_id と同じくセーブ・ロードの記録再生とも噛み合う。
   */
  private nextHandle = 1;

  /** 関数を預かり、その番号を返す。 */
  retain(f: Handler): number {
    const handle = this.nextHandle++;
    this.entries.set(handle, f);
    return handle;
  }

  /** 番号から関数を引く。手放された (または知らない) 番号なら `undefined`。 */
  get(handle: number): Handler | undefined {
    return this.entries.get(handle);
  }

  /** 関数を手放す。知らない番号は無視する (二重に手放しても害は無い)。 */
  release(handles: Iterable<number>): void {
    for (const handle of handles) {
      this.entries.delete(handle);
    }
  }

  /** 預かっている数。 */
  get size(): number {
    return this.entries.size;
  }
}
