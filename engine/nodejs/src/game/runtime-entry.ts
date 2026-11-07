// ビルド済みの Web runtime (`npm run build:runtime`) のエントリ。`src/game/entry.ts` の代わりに使う
// (vite.config.ts の `runtime` モード)。
//
// ゲームを焼き込まず、ページの横に置かれたゲームのデータを実行時に読む。`biwa build` がそれを置く:
//   biwa-game.json   ゲームの情報 (名前・タイトル・buildId)
//   game.wasm        コンパイラの出力
//   assets/          パッケージのアセット
// TypeScript のターゲット (deprecated) は扱わない。
import type { BiwaBackend } from "../engine/game";

const GAME_INFO = "biwa-game.json";
const GAME_WASM = "game.wasm";

/** `biwa-game.json` のうち、ここで使うもの (`cli/src/build.rs` の `write_game_info`)。 */
interface GameInfo {
  name?: string;
  title?: string;
  /** wasm の内容から決まる値。再ビルドで同じ URL のまま中身が変わるので、キャッシュを避けるのに使う。 */
  buildId?: string;
}

async function load(): Promise<BiwaBackend> {
  // 再ビルドのたびに書き換わるので、キャッシュを使わない。
  const response = await fetch(GAME_INFO, { cache: "no-store" });
  if (!response.ok) {
    throw new Error(
      `[biwa] failed to load ${GAME_INFO} (${response.status}): this page needs the game data next to it (built by \`biwa build\`)`,
    );
  }
  const info = (await response.json()) as GameInfo;
  return {
    kind: "wasm",
    packageName: info.title ?? info.name ?? "Biwa",
    url: GAME_WASM,
    buildId: info.buildId ?? String(Date.now()),
  };
}

const backend: BiwaBackend = await load();

export default backend;
