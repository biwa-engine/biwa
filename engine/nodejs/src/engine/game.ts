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
  window: {
    canvas: unknown;
    message_window: unknown;
  };
}

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
 * ゲーム開始時の `Game` はゲーム側の `fn on_new_game() -> Game[..]` が組み立てる。
 * エンジンが組み立てられないのは、`characters` と `states` の型を
 * ゲーム開発者が決めるからである
 * (wasm ではさらに、`Game` が JS から作れない WasmGC の struct でもある)。
 */
export type BiwaOnNewGame = () => BiwaGame;

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
