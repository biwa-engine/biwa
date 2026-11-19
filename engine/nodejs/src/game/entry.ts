// ゲームの読み込み口。Web runtime (`npm run build:runtime`) はゲームを焼き込まず、ここで実行時に読む。
//
// ゲームを焼き込まず、ページの横に置かれたゲームのデータを実行時に読む。`biwa build` がそれを置く:
//   biwa-game.json   ゲームの情報 (名前・タイトル・buildId)
//   game.wasm        コンパイラの出力
//   assets/          パッケージのアセット
// TypeScript のターゲット (deprecated) は扱わない。
//
// `biwa dev --platform web` も同じ runtime を CLI に内蔵した HTTP サーバで配る (`cli/src/serve.rs`)。そのときは `biwa-game.json` に
// `dev: true` が付き、建て直しの知らせ (`__biwa_dev/events`、Server-Sent Events) を受けてページを読み直す。
import type { BiwaBackend } from "../engine/game";

const GAME_INFO = "biwa-game.json";
const GAME_WASM = "game.wasm";

/** `biwa-game.json` のうち、ここで使うもの (`cli/src/build.rs` の `write_game_info`)。 */
interface GameInfo {
  name?: string;
  title?: string;
  /** wasm の内容から決まる値。再ビルドで同じ URL のまま中身が変わるので、キャッシュを避けるのに使う。 */
  buildId?: string;
  /** `biwa dev` から配られている (配布物には付かない)。 */
  dev?: boolean;
}

/** `biwa dev` の建て直しの知らせ (`cli/src/serve.rs`)。 */
const DEV_EVENTS = "__biwa_dev/events";

/** `biwa dev` が建て直したら (またはアセットが変わったら) ページを読み直す。 */
function listenForReload(): void {
  const events = new EventSource(DEV_EVENTS);
  events.addEventListener("reload", () => {
    events.close();
    location.reload();
  });
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
  if (info.dev === true) {
    listenForReload();
  }
  return {
    kind: "wasm",
    packageName: info.title ?? info.name ?? "Biwa",
    url: GAME_WASM,
    buildId: info.buildId ?? String(Date.now()),
  };
}

const backend: BiwaBackend = await load();

export default backend;
