// このファイルは `biwa dev` がコンパイル結果から生成して上書きする。
// リポジトリに置いてあるこれは、エンジン単体で `npm run dev` したときのためのプレースホルダ。
import type { BiwaEntrypoint } from "../engine/game";

export const packageName = "(no game)";

const entrypoint: BiwaEntrypoint = (game) => {
  console.warn(
    "[biwa] no game is loaded: run `biwa dev` in a Biwa package to generate src/game/entry.ts",
  );
  return game;
};

export default entrypoint;
