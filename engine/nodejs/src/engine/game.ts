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
 * ゲーム開始時の `Game` を組み立てる。
 *
 * `Window` / `Canvas` / `MessageWindow` は std 側では空の構造体で、
 * 実体はすべて syscall の向こうにある。
 * `characters` と `states` はゲーム側が初期化する手段がまだ無いので空で渡す。
 */
export function createInitialGame(name: string): BiwaGame {
  return {
    name,
    characters: {},
    states: {},
    window: {
      canvas: {},
      message_window: {},
    },
  };
}
