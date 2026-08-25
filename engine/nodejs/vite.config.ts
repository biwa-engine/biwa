import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";

const engineDir = fileURLToPath(new URL("./src/engine", import.meta.url));

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
  },
});
