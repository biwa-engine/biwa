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
  ENTER_SCENE_PAGE,
  FLUSH_AFTER_CAST,
  RUNTIME_NAMESPACE,
} from "./contract";
import { HandlerTable } from "../handlerTable";
import type { SceneStartRequest } from "../../ui/UIObjects";

/** メインスレッドから来る起動指示。 */
export interface StartMessage {
  kind: "start";
  /** 生成物 (`.wasm`) の URL。 */
  url: string;
  /** ブロッキング syscall の結果を受け取る共有バッファ。 */
  buffer: SharedArrayBuffer;
}

/**
 * scene を始めてほしいという知らせ。
 *
 * SceneStartButton が押されたときにメインスレッドが送る。中身は `UIObjects` の `SceneStartRequest`
 * (預けた関数の番号と、ScenePage の出力先の ui_id)。
 */
export interface StartSceneMessage extends SceneStartRequest {
  kind: "startScene";
}

/**
 * ハンドラを手放してよいという知らせ。
 *
 * 持ち主の Element が消えたときにメインスレッドが送る (`UIObjects` の `destroy`)。
 * Worker はその番号の関数を表から外す (`docs/host-function-values.md`)。
 */
export interface ReleaseMessage {
  kind: "release";
  handles: number[];
}

type HostMessage = StartMessage | StartSceneMessage | ReleaseMessage;

/** DOM の型と衝突させずに Worker のグローバルを触るための最小の窓口。 */
interface WorkerScope {
  postMessage(message: WorkerMessage): void;
  addEventListener(
    type: "message",
    listener: (event: { data: HostMessage }) => void,
  ): void;
}

const scope = globalThis as unknown as WorkerScope;

/**
 * コンパイラがエントリポイント (`fn main()`) に付ける固定の名前。
 *
 * ランタイムが名前で呼ぶゲーム側の関数はこれだけである。scene も最初の `Game` の作り方も、
 * 関数の値として UI (`Window` / `SceneStartButton`) から預かる。
 */
const ENTRYPOINT = "__biwa_entrypoint";

/**
 * std が `GameWindow` を組み立てる入口として host export している名前。
 * (`[[host_export="__biwa_std_game_window_new"]]`、`library/std/src/game/ui.biwa`)
 */
const GAME_WINDOW_NEW = "__biwa_std_game_window_new";

const decoder = new TextDecoder();

/** 届いた scene 開始の知らせ。まだ誰も待っていなければここに置いておく。 */
let sceneMessage: StartSceneMessage | null = null;
let sceneWaiter: ((message: StartSceneMessage) => void) | null = null;

scope.addEventListener("message", (event) => {
  const message = event.data;
  switch (message.kind) {
    case "start":
      void start(message);
      return;
    case "startScene":
      // 2 回目以降の scene の開始は未定義 (§19 の R8)。メインスレッドは 1 回しか送らない。
      if (sceneWaiter !== null) {
        sceneWaiter(message);
        sceneWaiter = null;
      } else {
        sceneMessage = message;
      }
      return;
    case "release":
      // 呼び出し中 (scene の実行中) はイベントループに帰らないので、ここに来るのは
      // Worker が Biwa のコードを実行していないときだけである。
      handlers.release(message.handles);
      return;
  }
});

/** scene 開始の知らせを待つ。 */
function waitForScene(): Promise<StartSceneMessage> {
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
  const entrypoint = exported(instance, ENTRYPOINT);
  const gameWindowNew = exported(instance, GAME_WINDOW_NEW);

  channel.report({ kind: "ready" });

  // 1. UI を出す。UI はすべてゲーム側 (`fn main()`) が決め、その中で `Window` を `show()` する。
  //    UI の syscall は止まらない (まとめて流す) ので、ここで流し切っておく。
  entrypoint();
  channel.flush();

  // 2. SceneStartButton が押されるまで待つ。
  //    Worker はその間イベントループに帰る (Biwa のコードを実行していない) ので、
  //    メインスレッドからの知らせを受け取れ、預かった関数を呼んでよい (`docs/host-function-values.md`)。
  const request = await waitForScene();
  const onClick = handlerOf(request.onClick, "on_click");
  const mainScene = handlerOf(request.mainScene, "main_scene");

  // 3. scene を始める。`Game` も `GameWindow` も JS からは組み立てられないので、
  //    出力先の束 `GameWindow` は std の host export に作らせ、ボタンの `on_click` に `Game[S]` を作らせる。
  //    ScenePage を見せてから、その `Game[S]` で Window の `main_scene` を始める。
  //    値はすべて中身を見ずに受け渡す (`S` はホストに見えない)。
  const game = onClick(
    gameWindowNew(request.canvasId, request.messageAreaId),
  );
  channel.cast(ENTER_SCENE_PAGE, [request.windowId]);
  mainScene(game);
}

/** 預かった関数を引く。手放されていれば名前を添えて叱る。 */
function handlerOf(
  handle: number,
  what: string,
): (...args: unknown[]) => unknown {
  const f = handlers.get(handle);
  if (f === undefined) {
    throw new Error(`[biwa] the ${what} handler (#${handle}) is not retained`);
  }
  return f;
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
 * Biwa から預かった関数 (ハンドラ) の表 (`contract.ts` の `retain`)。
 *
 * 関数はスレッドを越えられないので、ここに置いてメインスレッドには番号だけを送る。
 * ホストが関数を呼ぶ (ホスト → Biwa) のもこの Worker の中からで、
 * 呼んでよいのは Worker が Biwa のコードを実行していないときだけである
 * (`docs/host-function-values.md`)。今呼ぶのは SceneStartButton による scene の開始だけである (`run`)。
 */
const handlers = new HandlerTable();

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

  if (kind === "retain") {
    // 関数は表に預けて番号に替える。
    return ((...args: unknown[]): void => {
      const sent = args.map((arg) =>
        typeof arg === "function"
          ? handlers.retain(arg as (...a: unknown[]) => unknown)
          : arg,
      );
      channel.cast(name, sent);
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
