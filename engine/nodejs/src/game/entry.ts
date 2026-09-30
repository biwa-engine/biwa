// このファイルは `biwa dev` がコンパイル結果から生成して上書きする。
// リポジトリに置いてあるこれは、エンジン単体で `npm run dev` したときのためのプレースホルダ。
import type {
  BiwaApp,
  BiwaBackend,
  BiwaEntrypoint,
  BiwaGameWindowNew,
  BiwaOnNewGame,
  BiwaWindowShow,
} from "../engine/game";

// UI は何も出さない。scene を映す Page も無いので、entrypoint まで進むことは無い。
const app: BiwaApp = () => {
  console.warn(
    "[biwa] no game is loaded: run `biwa dev` in a Biwa package to generate src/game/entry.ts",
  );
  return null;
};

const windowShow: BiwaWindowShow = () => 0;

const entrypoint: BiwaEntrypoint = function*(game) {
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
  app,
  windowShow,
};

export default backend;
