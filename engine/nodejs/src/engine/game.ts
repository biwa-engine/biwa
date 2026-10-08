/**
 * コンパイラ生成コードとエンジンの境界の型。
 *
 * 生成された TypeScript の型はマングル名で、パッケージごとに変わるため、
 * エンジンからは構造だけを見る。
 * `characters` と `states` はゲーム開発者が定義する型なので、
 * エンジンは中身を知らない。
 */
export interface BiwaGame {
  name: string;
  characters: unknown;
  states: unknown;
  window: BiwaGameWindow;
}

/**
 * std の `GameWindow` (出力先の Canvas / MessageArea の ui_id の束)。
 *
 * エンジンは中身を見ない。`__biwa_std_game_window_new` で作り、
 * そのまま `on_new_game` に渡すだけである
 * (wasm では JS から作れない WasmGC の struct でもある)。
 */
export interface BiwaGameWindow {
  canvas: unknown;
  message_area: unknown;
}

/**
 * std が host export する `__biwa_std_game_window_new` の型。
 *
 * 引数は出力先の UI Element (`Canvas` / `MessageArea`) の ui_id。
 * **0 は「無し」**を表す (std 側で `None` に読み替える。ui_id は 1 から振られる)。
 */
export type BiwaGameWindowNew = (
  canvasId: number,
  messageAreaId: number,
) => BiwaGameWindow;

/**
 * `__biwa_app` の型。ゲーム側の `fn app()`。
 *
 * ランタイムは起動時にまずこれを呼ぶ。ゲーム側がその中で `Window[S]` を組み立てて
 * `show()` し、UI の syscall が出る。UI はすべてゲーム側が決める (エンジンは既定の UI を置かない)。
 * 戻り値が無いのは、`Window[S]` の `S` (ゲームの状態の型) をホストに見せないためである。
 */
export type BiwaApp = () => void;

/**
 * `__biwa_entrypoint` の型。
 *
 * scene は generator function として出力される (`scene main` のシグネチャ
 * `(g: Game) -> Game` に対応)。呼んでも本体は走らず、
 * kernel が `next()` で駆動して初めて進む。
 */
export type BiwaEntrypoint = (
  game: BiwaGame,
) => Generator<unknown, BiwaGame, unknown>;

/**
 * `__biwa_on_new_game` の型。
 *
 * ゲーム開始時の `Game` はゲーム側の `fn on_new_game(window: GameWindow) -> Game[..]`
 * が組み立てる。エンジンが組み立てられないのは、`characters` と `states` の型を
 * ゲーム開発者が決めるからである
 * (wasm ではさらに、`Game` が JS から作れない WasmGC の struct でもある)。
 * 出力先の束 `window` だけはエンジンが用意して渡す。
 */
export type BiwaOnNewGame = (window: BiwaGameWindow) => BiwaGame;

/**
 * ゲーム本体の受け渡し方。`biwa dev` が生成する `src/game/entry.ts` の形である。
 *
 * コンパイラは同じソースから TypeScript にも wasm にも吐けるので、
 * エンジンはどちらで来ても動く必要がある。
 * 違うのは実行のさせ方だけで、エンジン API (`api/*`) は共有している。
 */
export type BiwaBackend =
  | {
    /** 生成物が TypeScript。scene は generator で、kernel が `next()` で駆動する。 */
    kind: "typescript";
    packageName: string;
    entrypoint: BiwaEntrypoint;
    onNewGame: BiwaOnNewGame;
    /** std の `__biwa_std_game_window_new` (playable package のモジュールから再 export されている)。 */
    gameWindowNew: BiwaGameWindowNew;
    /** ゲーム側の `fn app()`。 */
    app: BiwaApp;
  }
  | {
    /** 生成物が wasm。Worker で走らせ、syscall はスレッドを跨ぐ。 */
    kind: "wasm";
    packageName: string;
    /** `.wasm` の URL。 */
    url: string;
    /** 生成物の内容から決まる値。ブラウザのキャッシュを避けるために付ける。 */
    buildId: string;
  };
