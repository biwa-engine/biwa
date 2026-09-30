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
export interface StartMessage {
  kind: "start";
  /** 生成物 (`.wasm`) の URL。 */
  url: string;
  /** ブロッキング syscall の結果を受け取る共有バッファ。 */
  buffer: SharedArrayBuffer;
}

/**
 * scene を始めてよいという知らせ。
 *
 * `app()` の Window の `scene_page_id` の Page に遷移したときにメインスレッドが送る。
 * 値はその Page の `canvas` / `message_area` から引いた出力先の ui_id (0 は「無し」)。
 */
export interface SceneMessage {
  kind: "scene";
  canvasId: number;
  messageAreaId: number;
}

type HostMessage = StartMessage | SceneMessage;

/** DOM の型と衝突させずに Worker のグローバルを触るための最小の窓口。 */
interface WorkerScope {
  postMessage(message: WorkerMessage): void;
  addEventListener(
    type: "message",
    listener: (event: { data: HostMessage }) => void,
  ): void;
}

const scope = globalThis as unknown as WorkerScope;

/** コンパイラがエントリポイントに付ける固定の名前。 */
const ENTRYPOINT = "__biwa_entrypoint";

/** コンパイラが初期 `Game` の組み立てに付ける固定の名前。 */
const NEW_GAME = "__biwa_on_new_game";

/** コンパイラが UI の root の組み立て (`fn app()`) に付ける固定の名前。 */
const APP = "__biwa_app";

/**
 * std が `GameWindow` を組み立てる入口として host export している名前。
 * (`[[host_export="__biwa_std_game_window_new"]]`、`library/std/src/game/ui.biwa`)
 */
const GAME_WINDOW_NEW = "__biwa_std_game_window_new";

/**
 * std が `Window` を表示する入口として host export している名前。
 * (`[[host_export="__biwa_std_window_show"]]`、`library/std/src/game/ui/window.biwa`)
 */
const WINDOW_SHOW = "__biwa_std_window_show";

const decoder = new TextDecoder();

/** 届いた scene 開始の知らせ。まだ誰も待っていなければここに置いておく。 */
let sceneMessage: SceneMessage | null = null;
let sceneWaiter: ((message: SceneMessage) => void) | null = null;

scope.addEventListener("message", (event) => {
  const message = event.data;
  switch (message.kind) {
    case "start":
      void start(message);
      return;
    case "scene":
      // 2 回目以降の遷移で何をするかは未定義 (セーブ・ロードが整っていないため)。
      // メインスレッドは 1 回しか送らない。
      if (sceneWaiter !== null) {
        sceneWaiter(message);
        sceneWaiter = null;
      } else {
        sceneMessage = message;
      }
      return;
  }
});

/** scene 開始の知らせを待つ。 */
function waitForScene(): Promise<SceneMessage> {
  if (sceneMessage !== null) {
    return Promise.resolve(sceneMessage);
  }
  return new Promise((resolve) => {
    sceneWaiter = resolve;
  });
}

async function start(message: StartMessage): Promise<void> {
  const channel = new SyscallChannel(message.buffer, (m) =>
    scope.postMessage(m),
  );

  try {
    await run(message, channel);
    channel.report({ kind: "exit" });
  } catch (e) {
    channel.report({ kind: "error", message: describe(e) });
  }
}

async function run(
  { url }: StartMessage,
  channel: SyscallChannel,
): Promise<void> {
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

  // 足りないものがあれば、何かを始める前に名前で叱る。
  const app = exported(instance, APP);
  const windowShow = exported(instance, WINDOW_SHOW);
  const entrypoint = exported(instance, ENTRYPOINT);
  const newGame = exported(instance, NEW_GAME);
  const gameWindowNew = exported(instance, GAME_WINDOW_NEW);

  channel.report({ kind: "ready" });

  // 1. UI を出す。UI はすべてゲーム側 (`fn app() -> Window`) が決める。
  //    `Window` も WasmGC の struct なので、表示は std の host export に任せる。
  //    UI の syscall は止まらない (まとめて流す) ので、ここで流し切っておく。
  windowShow(app());
  channel.flush();

  // 2. Window の `scene_page_id` の Page に遷移するまで待つ。
  //    Worker はその間イベントループに帰るので、メインスレッドからの知らせを受け取れる。
  const { canvasId, messageAreaId } = await waitForScene();

  // 3. scene を始める。`Game` も `GameWindow` も JS からは組み立てられないので、
  //    出力先の束 `GameWindow` は std の host export に作らせ、
  //    それを渡してゲーム側の `fn on_new_game(window)` に `Game` を作らせる。
  entrypoint(newGame(gameWindowNew(canvasId, messageAreaId)));
}

/** 固定名の export を取り出す。無ければ名前を添えて叱る。 */
function exported(
  instance: WebAssembly.Instance,
  name: string,
): (...args: unknown[]) => unknown {
  const value = instance.exports[name];
  if (typeof value !== "function") {
    throw new Error(`the generated module does not export \`${name}\``);
  }
  return value as (...args: unknown[]) => unknown;
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

  sys_int_to_string: (value: number): string => String(value),

  // f32 として渡ってくるので、そのまま文字列にすると
  // `0.30000001192092896` のような桁が出る。f32 が表せる精度で丸める。
  sys_float_to_string: (value: number): string =>
    String(Number(value.toPrecision(9))),

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
 *
 * ui_id (`sys_ui_create`) もこの連番から振る (UI Element を作るのはゲーム側だけなので、
 * メインスレッドの採番と重なることは無い)。
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
