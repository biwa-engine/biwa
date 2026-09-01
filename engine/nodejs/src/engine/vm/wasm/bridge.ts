/**
 * Worker (wasm) とメインスレッド (エンジン) の間の syscall の受け渡し。
 *
 * 引数は `postMessage` で送る (構造化複製で足りる: 文字列と数値だけ)。
 * 戻り値だけは同期的に受け取る必要があるので `SharedArrayBuffer` を使う。
 *
 * ブロッキング syscall の流れ:
 *
 * ```
 *   Worker                         Main
 *   ------                         ----
 *   状態 = PENDING
 *   postMessage({name, args})  ->  ハンドラを実行 (await できる)
 *   Atomics.wait(状態)             結果を SAB に書く
 *        (スレッドが止まる)        状態 = DONE
 *                               <- Atomics.notify
 *   結果を読んで wasm に返る
 * ```
 *
 * 本物の syscall と同じで、呼んだ側からは「関数が長く掛かった」ようにしか見えない。
 * 止まるのは Worker だけなので、その間もメインスレッドは描画を続けられる。
 *
 * 止まらない syscall (`cast`) はまとめて 1 通で流す。「まとめ流し」を参照。
 */

/** 制御語の数 (Int32 単位)。 */
const CONTROL_WORDS = 2;

/** 制御語: 呼び出しの状態。 */
const STATE = 0;
/** 制御語: 結果の JSON のバイト長。 */
const RESULT_LEN = 1;

/** 結果を書ける最大バイト数。 */
const PAYLOAD_CAPACITY = 64 * 1024;

const HEADER_BYTES = CONTROL_WORDS * 4;

/**
 * 溜めておける cast の数。超えたら送り出す。
 *
 * ブロッキング syscall にも発火にも当たらないまま走り続けるシーンへの保険で、
 * 普段はここに掛からない。
 */
const CAST_BUFFER_LIMIT = 64;

/** 呼び出しの状態。`enum` は消去可能な構文ではないので定数にしてある。 */
const State = {
  /** メインスレッドが処理中。 */
  Pending: 0,
  /** 完了。結果が書かれている。 */
  Done: 1,
  /** ハンドラが失敗した。結果の位置にメッセージが入っている。 */
  Failed: 2,
} as const;

type State = (typeof State)[keyof typeof State];

/** 1 つの syscall。 */
export interface SyscallEntry {
  name: string;
  args: unknown[];
}

/** Worker からメインスレッドへ送る、止まらない syscall のまとまり。 */
export interface SyscallBatch {
  kind: "syscalls";
  calls: SyscallEntry[];
}

/** Worker からメインスレッドへ送る、結果を待つ syscall。 */
export interface SyscallRequest extends SyscallEntry {
  kind: "syscall";
}

/** Worker からメインスレッドへ送る、実行そのものの結末。 */
export type WorkerReport =
  | { kind: "ready" }
  | { kind: "exit" }
  | { kind: "error"; message: string };

export type WorkerMessage = SyscallBatch | SyscallRequest | WorkerReport;

export function createChannelBuffer(): SharedArrayBuffer {
  return new SharedArrayBuffer(HEADER_BYTES + PAYLOAD_CAPACITY);
}

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/**
 * Worker 側の窓口。
 *
 * `Atomics.wait` はメインスレッドでは使えないため、これは Worker 専用である。
 *
 * ## まとめ流し
 *
 * Worker はゲームの実行中にイベントループへ帰らない。生成物のエントリポイントを
 * **同期に呼び切る**からである (`worker.ts`)。したがって「ターンの終わり」に
 * 相当する瞬間は存在せず、cast を流す契機は自分で決めるしかない。
 *
 * - `call` の直前 (順序を保つため。必ず要る)
 * - 呼び出し側が区切ったとき (`contract.ts` の `FLUSH_AFTER_CAST`)
 * - 実行の結末を報告するとき
 * - 溜まりすぎたとき (保険)
 */
export class SyscallChannel {
  private readonly control: Int32Array;
  private readonly payload: Uint8Array;
  private readonly post: (message: WorkerMessage) => void;
  private buffered: SyscallEntry[] = [];

  constructor(
    buffer: SharedArrayBuffer,
    post: (message: WorkerMessage) => void,
  ) {
    this.control = new Int32Array(buffer, 0, CONTROL_WORDS);
    this.payload = new Uint8Array(buffer, HEADER_BYTES);
    this.post = post;
  }

  /** 積んで即座に返る syscall。まとめて後で流す。 */
  cast(name: string, args: unknown[]): void {
    this.buffered.push({ name, args });
    if (this.buffered.length >= CAST_BUFFER_LIMIT) {
      this.flush();
    }
  }

  /** 溜めてある cast を送り出す。 */
  flush(): void {
    if (this.buffered.length === 0) return;
    const calls = this.buffered;
    this.buffered = [];
    this.post({ kind: "syscalls", calls });
  }

  /** 完了するまで Worker を止める syscall。 */
  call(name: string, args: unknown[]): unknown {
    // 先に積んだものを追い越さないよう、必ず流してから止まる。
    this.flush();

    Atomics.store(this.control, STATE, State.Pending);
    this.post({ kind: "syscall", name, args });

    // 待っている間、この Worker は JS を 1 命令も実行しない。
    // メッセージも処理されないので、結果は必ず SAB 経由で受け取る。
    while (Atomics.load(this.control, STATE) === State.Pending) {
      Atomics.wait(this.control, STATE, State.Pending);
    }

    const length = Atomics.load(this.control, RESULT_LEN);
    const text =
      length === 0 ? "" : decoder.decode(this.payload.subarray(0, length));

    if (Atomics.load(this.control, STATE) === State.Failed) {
      throw new Error(`[biwa] syscall \`${name}\` failed: ${text}`);
    }

    return text === "" ? undefined : (JSON.parse(text) as unknown);
  }

  report(message: WorkerReport): void {
    this.flush();
    this.post(message);
  }
}

/**
 * メインスレッド側。ブロッキング syscall の結果を書き戻して Worker を起こす。
 *
 * `value` は JSON にできるものに限る。wasm の値 (externref など) は
 * そもそもスレッドを越えられないので、返せるのはデータだけである。
 */
export function completeCall(buffer: SharedArrayBuffer, value: unknown): void {
  finish(buffer, State.Done, value === undefined ? "" : JSON.stringify(value));
}

/** メインスレッド側。ハンドラが失敗したことを伝えて Worker を起こす。 */
export function failCall(buffer: SharedArrayBuffer, message: string): void {
  finish(buffer, State.Failed, message);
}

function finish(buffer: SharedArrayBuffer, state: State, text: string): void {
  const control = new Int32Array(buffer, 0, CONTROL_WORDS);
  const payload = new Uint8Array(buffer, HEADER_BYTES);

  const bytes = encoder.encode(text);
  if (bytes.length > payload.length) {
    // ここで諦めると Worker が永久に止まるので、失敗として起こす。
    const overflow = encoder.encode("the syscall result is too large");
    payload.set(overflow);
    Atomics.store(control, RESULT_LEN, overflow.length);
    Atomics.store(control, STATE, State.Failed);
  } else {
    payload.set(bytes);
    Atomics.store(control, RESULT_LEN, bytes.length);
    Atomics.store(control, STATE, state);
  }

  Atomics.notify(control, STATE);
}
