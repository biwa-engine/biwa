import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";

const engineApiDir = fileURLToPath(new URL("./src/engine/api", import.meta.url));

export default defineConfig({
  resolve: {
    alias: [
      // std の native TypeScript が書く論理パス `@biwa/engine/<name>` を
      // エンジンの syscall 層に解決する。
      // これにより、生成物がどこに置かれても import が壊れない。
      { find: /^@biwa\/engine\/(.*)$/, replacement: `${engineApiDir}/$1` },
    ],
  },
  server: {
    open: false,
  },
});
