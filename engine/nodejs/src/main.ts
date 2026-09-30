import { setEngineContext } from "./engine/api/context";
import { CanvasObjects } from "./engine/canvas/CanvasObjects";
import { CanvasSurfaces } from "./engine/canvas/CanvasSurfaces";
import type { BiwaBackend } from "./engine/game";
import { Renderer } from "./engine/Renderer";
import { type ScenePageEntry, UIObjects } from "./engine/ui/UIObjects";
import { Kernel } from "./engine/vm/kernel";
import { createSyscallTable } from "./engine/vm/handlers";
import { runWasm } from "./engine/vm/wasm/host";
import backend from "./game/entry";

const WIDTH = 1280;
const HEIGHT = 720;

// UI (Window/Page/Link/MessageArea/...) は canvas より前面に置く。
// Message Window も UI Element `MessageArea` の中身としてここに入る。
// クリック判定を奪うのは Link だけなので、下の演出やテキストを覆っても
// リンクの無い場所ではクリックがそのまま `host` まで抜ける。
const UI_LAYER_ID = "ui";

const host = document.querySelector<HTMLDivElement>("#app")!;
host.style.cursor = "pointer";

const renderer = new Renderer(host);
await renderer.init(WIDTH, HEIGHT);

// canvas に描くものの層は、出力先の `Canvas` Element ごとの描画先が
// `create_object` の layer index から必要に応じて作る。
// ここで定義するのは DOM レイヤーだけでよい。
renderer.layers.defineDom(UI_LAYER_ID, 25);

const ui = new UIObjects(renderer.layers.dom(UI_LAYER_ID));
const surfaces = new CanvasSurfaces(renderer.app.stage, host, ui);
const objects = new CanvasObjects(surfaces);
// エンジンが Ticker に登録するコールバックはこれ 1 つだけである。
// オブジェクトごとに生やさないのは、リークを避けるためでもあるし、
// ポーズ・オート・スキップを 1 箇所の時間操作で効かせるためでもある。
renderer.app.ticker.add((ticker) => {
  // canvas の描画先を `Canvas` Element の今の矩形に合わせてから射影する。
  surfaces.sync();
  objects.update(ticker.deltaMS);
  // 文字送りも同じ時計で進める (すべての MessageArea)。倍率を `objects` から借りるのは、
  // ポーズ・オート・スキップが 1 箇所の時間操作で効くようにするためである。
  ui.update(ticker.deltaMS * objects.timeScale);
});

// 以降、syscall の実装はこのコンテキストを通してエンジンを触る。
setEngineContext({
  renderer,
  objects,
  ui,
  host,
});

document.title = `${backend.packageName} — Biwa`;

// scene を映す Page (Window の `scene_page_id`) への最初の遷移。
// その Page の `canvas` / `message_area` が scene の出力先になる。
//
// 2 回目以降の遷移で何をするかは未定義である (セーブ・ロードが整っていないため)。
// いまは最初の 1 回で scene を始め、以降は何もしない (Promise は一度しか解決しない)。
const scenePage = new Promise<ScenePageEntry>((resolve) => {
  ui.onScenePageEntered(resolve);
});

// エンジンは UI を何も置かない。`app()` の Window が表示されて初めて画面に何かが出る。
await runGame(backend, scenePage);

/**
 * ゲームを走らせる。
 *
 * 流れはどちらのターゲットでも同じである:
 * `app()` の Window を表示 → `scene_page_id` の Page へ遷移するのを待つ →
 * その出力先で `GameWindow` を作り `on_new_game(window)` → `scene main`。
 *
 * どちらのターゲットでも、エンジンから見えるのは syscall の流れだけである。
 * 違うのは「どこで動いていて、どうやって中断するか」でしかない。
 */
async function runGame(
  backend: BiwaBackend,
  scenePage: Promise<ScenePageEntry>,
): Promise<void> {
  switch (backend.kind) {
    case "typescript": {
      backend.windowShow(backend.app());
      const { canvasId, messageAreaId } = await scenePage;

      // scene は generator なので、呼んだだけでは何も起きない。
      // kernel が next() で駆動し、yield された syscall を処理して結果を書き戻す。
      const kernel = new Kernel(createSyscallTable());
      const window = backend.gameWindowNew(canvasId, messageAreaId);
      await kernel.run(backend.entrypoint(backend.onNewGame(window)));
      return;
    }
    case "wasm": {
      // 生成物は Worker で走る。ブロッキング syscall は Worker のスレッドを止める。
      // buildId を付けるのは、再ビルドで同じ URL のまま中身が変わるためである。
      const url = new URL(backend.url, location.href);
      url.searchParams.set("v", backend.buildId);
      await runWasm(url.href, scenePage);
      return;
    }
  }
}
