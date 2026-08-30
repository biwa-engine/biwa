import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";

const engineDir = fileURLToPath(new URL("./src/engine", import.meta.url));

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

export default defineConfig({
  resolve: {
    alias: [
      // std の native TypeScript が書く論理パス `@biwa/engine/<path>` を
      // エンジン実装に解決する。
      // これにより、生成物がどこに置かれても import が壊れない。
      { find: /^@biwa\/engine\/(.*)$/, replacement: `${engineDir}/$1` },
    ],
  },
  server: {
    open: false,
    headers: CROSS_ORIGIN_ISOLATION,
  },
  preview: {
    headers: CROSS_ORIGIN_ISOLATION,
  },
});
