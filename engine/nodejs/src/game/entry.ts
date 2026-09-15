// このファイルは `biwa dev` がコンパイル結果から生成して上書きする。
// リポジトリに置いてあるこれは、エンジン単体で `npm run dev` したときのためのプレースホルダ。
import type {
  BiwaBackend,
  BiwaEntrypoint,
  BiwaOnNewGame,
} from "../engine/game";

const entrypoint: BiwaEntrypoint = function*(game) {
  console.warn(
    "[biwa] no game is loaded: run `biwa dev` in a Biwa package to generate src/game/entry.ts",
  );
  return game;
};

const onNewGame: BiwaOnNewGame = () => ({
  name: "(no game)",
  characters: {},
  states: {},
  window: { canvas: {}, message_window: {} },
});

const backend: BiwaBackend = {
  kind: "typescript",
  packageName: "(no game)",
  entrypoint,
  onNewGame,
};

export default backend;
