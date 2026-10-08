// このファイルは `biwa dev` がコンパイル結果から生成して上書きする。
// リポジトリに置いてあるこれは、エンジン単体で `npm run dev` したときのためのプレースホルダ。
import type {
  BiwaApp,
  BiwaBackend,
  BiwaGameWindowNew,
} from "../engine/game";

// UI は何も出さない。SceneStartButton も無いので、scene まで進むことは無い。
const app: BiwaApp = () => {
  console.warn(
    "[biwa] no game is loaded: run `biwa dev` in a Biwa package to generate src/game/entry.ts",
  );
};

const gameWindowNew: BiwaGameWindowNew = (canvasId, messageAreaId) => ({
  canvas: canvasId,
  message_area: messageAreaId,
});

const backend: BiwaBackend = {
  kind: "typescript",
  packageName: "(no game)",
  gameWindowNew,
  app,
};

export default backend;
