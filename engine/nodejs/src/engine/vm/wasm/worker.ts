/**
 * wasm 生成物を走らせる Worker。ゲームコードから見ればここが VM の中である。
 *
 * メインスレッドから分けてあるのは、ブロッキング syscall がこのスレッドを
 * `Atomics.wait` で止めるからである。止まっている間もメインスレッドは
 * 描画とイベント処理を続けられる。
 *
 * このファイルは Worker としてのみ読み込まれる。
 */

import { SyscallChannel, type WorkerMessage } from "./bridge";
import {
  ENGINE_NAMESPACE,
  ENGINE_SYSCALLS,
  FLUSH_AFTER_CAST,
  RUNTIME_NAMESPACE,
} from "./contract";

/** メインスレッドから来る起動指示。 */
interface StartMessage {
  kind: "start";
  /** 生成物 (`.wasm`) の URL。 */
  url: string;
  /** ブロッキング syscall の結果を受け取る共有バッファ。 */
  buffer: SharedArrayBuffer;
}

/** DOM の型と衝突させずに Worker のグローバルを触るための最小の窓口。 */
interface WorkerScope {
  postMessage(message: WorkerMessage): void;
  addEventListener(
    type: "message",
    listener: (event: { data: StartMessage }) => void,
    options?: { once?: boolean },
  ): void;
}

const scope = globalThis as unknown as WorkerScope;

/** コンパイラがエントリポイントに付ける固定の名前。 */
const ENTRYPOINT = "__biwa_entrypoint";

const decoder = new TextDecoder();

scope.addEventListener(
  "message",
  (event) => {
    void start(event.data);
  },
  { once: true },
);

async function start(message: StartMessage): Promise<void> {
  const channel = new SyscallChannel(message.buffer, (m) =>
    scope.postMessage(m),
  );

  try {
    await run(message.url, channel);
    channel.report({ kind: "exit" });
  } catch (e) {
    channel.report({ kind: "error", message: describe(e) });
  }
}

async function run(url: string, channel: SyscallChannel): Promise<void> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(
      `failed to fetch ${url}: ${response.status} ${response.statusText}`,
    );
  }
  const bytes = await response.arrayBuffer();

  const module = await WebAssembly.compile(bytes);

  // instance は import の実装から参照される (`string_const` が memory を読む) が、
  // その実装は instantiate に渡すので、この時点ではまだ存在しない。
  let instance: WebAssembly.Instance | null = null;
  const memory = (): Uint8Array => {
    const m = instance?.exports["memory"];
    if (!(m instanceof WebAssembly.Memory)) {
      throw new Error("the generated module does not export its memory");
    }
    return new Uint8Array(m.buffer);
  };

  const imports = buildImports(module, channel, memory);
  instance = await WebAssembly.instantiate(module, imports);

  const entrypoint = instance.exports[ENTRYPOINT];
  if (typeof entrypoint !== "function") {
    throw new Error(`the generated module does not export \`${ENTRYPOINT}\``);
  }

  channel.report({ kind: "ready" });

  // 引数は `Game` である。wasm の struct を JS から作る手段がまだ無いので
  // null を渡している。scene が `g` のフィールドを読むとここで trap する。
  // TODO: 初期 `Game` を wasm 側で組み立てる入口をコンパイラに用意する。
  (entrypoint as (game: unknown) => unknown)(null);
}

/**
 * 生成物が要求している import だけを組み立てる。
 *
 * 宣言を見てから埋めるのは、足りないときに名前で叱るためである。
 * import object に鍵が無いと `LinkError` になるが、そこには
 * 「どの機能をエンジンが実装していないのか」が出てこない。
 */
function buildImports(
  module: WebAssembly.Module,
  channel: SyscallChannel,
  memory: () => Uint8Array,
): WebAssembly.Imports {
  const runtime = runtimeImports(memory);
  const imports: WebAssembly.Imports = {};
  const missing: string[] = [];

  for (const declared of WebAssembly.Module.imports(module)) {
    const namespace = (imports[declared.module] ??= {});
    if (declared.name in namespace) {
      continue;
    }

    if (declared.module === RUNTIME_NAMESPACE) {
      const impl = runtime[declared.name];
      if (impl === undefined) {
        missing.push(`${declared.module} ${declared.name}`);
        continue;
      }
      namespace[declared.name] = impl;
      continue;
    }

    if (declared.module === ENGINE_NAMESPACE) {
      const kind = ENGINE_SYSCALLS[declared.name];
      if (kind === undefined) {
        missing.push(`${declared.module} ${declared.name}`);
        continue;
      }
      namespace[declared.name] = syscall(declared.name, kind, channel);
      continue;
    }

    missing.push(`${declared.module} ${declared.name}`);
  }

  if (missing.length > 0) {
    throw new Error(
      `the engine does not implement: ${missing.join(", ")} ` +
      "(the standard library and the engine are out of sync)",
    );
  }

  return imports;
}

/**
 * コンパイラ自身が要求する import。
 *
 * `string_const` は文字列リテラルの実体化である。生成物は文字列を
 * externref (= JS の文字列) として持つので、データセグメント上の
 * UTF-8 を JS 文字列に変える手段がホスト側に要る。
 */
function runtimeImports(
  memory: () => Uint8Array,
): Record<string, WebAssembly.ImportValue> {
  return {
    string_const: (offset: number, length: number): string =>
      decoder.decode(memory().subarray(offset, offset + length)),
  };
}

/** Worker 内で完結する syscall の実装。 */
const LOCAL_SYSCALLS: Record<string, (...args: never[]) => unknown> = {
  sys_string_concat: (a: string, b: string): string => a + b,

  sys_vec_new: (): unknown[] => [],

  sys_vec_of: (value: unknown): unknown[] => [value],

  sys_vec_push: (vec: unknown[], value: unknown): void => {
    vec.push(value);
  },

  sys_vec_len: (vec: unknown[]): number => vec.length,

  // 範囲外は Option::none (null) として返す。
  sys_vec_get: (vec: unknown[], index: number): unknown =>
    index < vec.length ? vec[index] : null,

  sys_map_insert: (
    map: Map<unknown, unknown>,
    key: unknown,
    value: unknown,
  ): void => {
    map.set(key, value);
  },

  // Option[V] は「値または null」なので、未登録の undefined を null に均す。
  //
  // NOTE: std の宣言では戻り値が anyref だが、実際に入っている値は
  // externref (JS の値) である。std 側の食い違いで、今のところ到達しない。
  sys_map_get: (map: Map<unknown, unknown>, key: unknown): unknown => {
    const value = map.get(key);
    return value === undefined ? null : value;
  },
};

/**
 * canvas オブジェクトの id を採る連番。
 *
 * メインスレッドと往復せずに `create_object` の戻り値を返すため、
 * 採番は Worker 側で行う (`contract.ts` の `alloc`)。
 * 単調増加なので決定的で、セーブ・ロードの記録再生とも噛み合う。
 */
let nextObjectId = 1;

/**
 * `biwa:engine` の import 1 つを、その区分に応じた関数にする。
 *
 * wasm 側から見ればどれも同じ同期呼び出しで、
 * 積んで返るのか完了まで待つのかは見えない。
 */
function syscall(
  name: string,
  kind: (typeof ENGINE_SYSCALLS)[string],
  channel: SyscallChannel,
): WebAssembly.ImportValue {
  if (kind === "local") {
    const impl = LOCAL_SYSCALLS[name];
    if (impl === undefined) {
      throw new Error(
        `[biwa] syscall \`${name}\` is declared local but has no implementation`,
      );
    }
    return impl as WebAssembly.ImportValue;
  }

  if (kind === "cast") {
    const flush = FLUSH_AFTER_CAST.has(name);
    return ((...args: unknown[]): void => {
      channel.cast(name, args);
      if (flush) channel.flush();
    }) as WebAssembly.ImportValue;
  }

  if (kind === "alloc") {
    // 採番だけをこちらで行い、本体はメインスレッドへ投げる。
    // 戻り値があるのに Worker は止まらない。
    return ((...args: unknown[]): number => {
      const id = nextObjectId++;
      channel.cast(name, [id, ...args]);
      return id;
    }) as WebAssembly.ImportValue;
  }

  return ((...args: unknown[]): unknown =>
    channel.call(name, args)) as WebAssembly.ImportValue;
}

function describe(e: unknown): string {
  if (e instanceof Error) {
    return e.stack ?? e.message;
  }
  return String(e);
}
