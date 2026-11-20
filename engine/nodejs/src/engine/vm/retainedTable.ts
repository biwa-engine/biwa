/**
 * ホストが預かる Biwa の値の表。
 *
 * Biwa から syscall の引数で渡された関数 (ハンドラ) や値 (struct) を、番号 (handle) と引き換えに預かる。
 * 規定は `docs/host-function-values.md` にある。
 *
 * Worker が持つ。関数 (wasm の関数のラッパー) も WasmGC の値も `postMessage` できず、
 * UI の DOM があるメインスレッドには番号だけを送る。関数と値は同じ番号の空間を使うので、
 * メインスレッドは手放すときにどちらかを区別しなくてよい。
 *
 * 表が握っている間はその値は GC されない。項目は持ち主の Element が
 * 消えるときに手放す (`UIObjects`)。
 */

/**
 * 預かる関数。
 *
 * 引数と戻り値の中身はホストからは見えない (wasm の GC 参照・JS の値)。
 * どの形で呼ぶかはハンドラの種類 (`api/ui.ts` の `HandlerKind`) ごとに決まっている。
 */
export type Handler = (...args: unknown[]) => unknown;

export class RetainedTable {
  /** 預かっているもの。関数 (`Handler`) か、中身の見えない値 (WasmGC の struct)。 */
  private readonly entries = new Map<number, unknown>();
  /**
   * 次に振る番号。0 は「無し」に取っておく。
   *
   * 単調増加なので決定的で、ui_id と同じくセーブ・ロードの記録再生とも噛み合う。
   */
  private nextHandle = 1;

  /** 関数か値を預かり、その番号を返す。 */
  retain(value: unknown): number {
    const handle = this.nextHandle++;
    this.entries.set(handle, value);
    return handle;
  }

  /** 番号から関数を引く。手放された (または知らない) 番号や、関数でないものなら `undefined`。 */
  handler(handle: number): Handler | undefined {
    const value = this.entries.get(handle);
    return typeof value === "function" ? (value as Handler) : undefined;
  }

  /** 番号から値を引く。手放された (または知らない) 番号なら `undefined`。 */
  value(handle: number): unknown {
    return this.entries.get(handle);
  }

  /** 番号が預かっているものを指しているか。 */
  has(handle: number): boolean {
    return this.entries.has(handle);
  }

  /** 手放す。知らない番号は無視する (二重に手放しても害は無い)。 */
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
