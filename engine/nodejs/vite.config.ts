import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";

const engineDir = fileURLToPath(new URL("./src/engine", import.meta.url));
const runtimeEntry = fileURLToPath(new URL("./src/game/runtime-entry.ts", import.meta.url));

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
 * `runtime` モード (`npm run build:runtime`) は、ゲームを焼き込まない「ビルド済みの Web runtime」を作る。
 *
 * ゲームの読み込み口 `./game/entry` (`biwa dev` が生成するスタブ) を `src/game/runtime-entry.ts`
 * (ページの横の `biwa-game.json` と `game.wasm` を実行時に読む) に差し替え、どこに置いても動くよう
 * `base` を `./` にする。出力 (`dist-runtime/`) は Biwa の CI がツールチェインに入れ、
 * `biwa build --platform web` などがゲームのデータを添えて配る (Node.js は要らない)。
 */
export default defineConfig(({ mode }) => {
  const runtime = mode === "runtime";
  return {
    base: runtime ? "./" : undefined,
    resolve: {
      alias: [
        // std の native TypeScript が書く論理パス `@biwa/engine/<path>` を
        // エンジン実装に解決する。
        // これにより、生成物がどこに置かれても import が壊れない。
        { find: /^@biwa\/engine\/(.*)$/, replacement: `${engineDir}/$1` },
        ...(runtime ? [{ find: /^\.\/game\/entry$/, replacement: runtimeEntry }] : []),
      ],
    },
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
