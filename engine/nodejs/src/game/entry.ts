// このファイルは `biwa dev` がコンパイル結果から生成して上書きする。
// リポジトリに置いてあるこれは、エンジン単体で `npm run dev` したときのためのプレースホルダ。
import type {
  BiwaBackend,
  BiwaEntrypoint,
  BiwaGameWindowNew,
  BiwaOnNewGame,
} from "../engine/game";

const entrypoint: BiwaEntrypoint = function*(game) {
  console.warn(
    "[biwa] no game is loaded: run `biwa dev` in a Biwa package to generate src/game/entry.ts",
  );
  return game;
};

const gameWindowNew: BiwaGameWindowNew = (canvasId, messageAreaId) => ({
  canvas: canvasId,
  message_area: messageAreaId,
});

const onNewGame: BiwaOnNewGame = (window) => ({
  name: "(no game)",
  characters: {},
  states: {},
  window,
});

const backend: BiwaBackend = {
  kind: "typescript",
  packageName: "(no game)",
  entrypoint,
  onNewGame,
  gameWindowNew,
};

export default backend;
