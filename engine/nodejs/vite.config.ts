import { defineConfig } from "vite";

/**
 * wasm 生成物を走らせるには `SharedArrayBuffer` が要る。
 *
 * ブロッキング syscall は Worker のスレッドを `Atomics.wait` で止めて、
 * メインスレッドが結果を書き戻して起こす。その受け渡しに使う。
 * ブラウザは cross-origin isolation されたページにしか
 * `SharedArrayBuffer` を渡さないので、この 2 つのヘッダが必要になる。
 *
 * 配信する側でも同じヘッダを付ける必要がある。
 */
const CROSS_ORIGIN_ISOLATION = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

/**
 * `runtime` モード (`npm run build:runtime` / `npm run watch:runtime`) は、ゲームを焼き込まない
 * 「ビルド済みの Web runtime」を `dist-runtime/` に作る。ゲームはページの横の `biwa-game.json` と `game.wasm` から
 * 実行時に読む (`src/game/entry.ts`)。どこに置いても動くよう `base` を `./` にする。
 *
 * 出力は Biwa の CI がツールチェインに入れ、`biwa build --platform web` などがゲームのデータを添えて配る
 * (ゲームを作る人の手元で Node.js は要らない)。エンジンを開発するときは、`npm run watch:runtime` の出力を
 * `biwa dev --platform web --web-runtime <this>/dist-runtime` で使う。
 */
export default defineConfig(({ mode }) => {
  const runtime = mode === "runtime";
  return {
    base: runtime ? "./" : undefined,
    build: {
      // 既定の `assets` は、パッケージの `assets/` を公開する位置 (`<base>assets/`) とぶつかる。
      // Vite が出すもの (JS・wasm・Worker) はゲームのアセットと混ざらない名前の下に置く。
      assetsDir: "_biwa",
      outDir: runtime ? "dist-runtime" : undefined,
      emptyOutDir: true,
    },
    server: {
      open: false,
      headers: CROSS_ORIGIN_ISOLATION,
    },
    preview: {
      headers: CROSS_ORIGIN_ISOLATION,
    },
  };
});
