import { setEngineContext } from "./engine/api/context";
import { lookupHandler, releaseHandlers } from "./engine/api/handler";
import { CanvasObjects } from "./engine/canvas/CanvasObjects";
import { CanvasSurfaces } from "./engine/canvas/CanvasSurfaces";
import type { BiwaBackend } from "./engine/game";
import { Renderer } from "./engine/Renderer";
import { type SceneStartRequest, UIObjects } from "./engine/ui/UIObjects";
import { type BiwaScene, Kernel } from "./engine/vm/kernel";
import { createSyscallTable } from "./engine/vm/handlers";
import { runWasm } from "./engine/vm/wasm/host";
import backend from "./game/entry";
import "./style.css";

// UI (Window/Page/Link/Canvas/MessageArea/...) を載せる層。
// canvas の描画先も Message Window も、それぞれ UI Element の中身としてここに入る。
// クリック判定を奪うのは Link だけなので、下の演出やテキストを覆っても
// リンクの無い場所ではクリックがそのまま `host` まで抜ける。
const UI_LAYER_ID = "ui";

const host = document.querySelector<HTMLDivElement>("#app")!;
host.style.cursor = "pointer";

const renderer = new Renderer(host);
// 画面全体を host にする。UI の範囲はすべて Biwa Language 側が決める。
renderer.init();

// canvas に描くものの層は、出力先の `Canvas` Element ごとの描画先が
// `create_object` の layer index から必要に応じて作る。
// ここで定義するのは DOM レイヤーだけでよい。
renderer.layers.defineDom(UI_LAYER_ID, 25);

const ui = new UIObjects(renderer.layers.dom(UI_LAYER_ID));
const surfaces = new CanvasSurfaces(ui);
const objects = new CanvasObjects(surfaces);
// エンジンが Ticker に登録するコールバックはこれ 1 つだけである。
// オブジェクトごとに生やさないのは、リークを避けるためでもあるし、
// ポーズ・オート・スキップを 1 箇所の時間操作で効かせるためでもある。
renderer.ticker.add((ticker) => {
  objects.update(ticker.deltaMS);
  // 文字送りも同じ時計で進める (すべての MessageArea)。倍率を `objects` から借りるのは、
  // ポーズ・オート・スキップが 1 箇所の時間操作で効くようにするためである。
  ui.update(ticker.deltaMS * objects.timeScale);
  // 射影し終えたものを、Canvas Element ごとの描画先に描く。
  surfaces.render();
});

// 以降、syscall の実装はこのコンテキストを通してエンジンを触る。
setEngineContext({
  renderer,
  objects,
  ui,
  host,
});

document.title = `${backend.packageName} — Biwa`;

// エンジンは UI を何も置かない。`fn main()` が Window を表示して初めて画面に何かが出る。
await runGame(backend);

/**
 * ゲームを走らせる。
 *
 * 流れはどちらのターゲットでも同じである:
 * `fn main()` で Window を表示 → SceneStartButton が押されるのを待つ →
 * Window の ScenePage の出力先で `GameWindow` を作り、ボタンの `on_click(window)` で `Game[S]` を作る →
 * ScenePage を見せる → Window の `main_scene(game)`。
 *
 * どちらのターゲットでも、エンジンから見えるのは syscall の流れだけである。
 * 違うのは「どこで動いていて、どうやって中断するか」でしかない。
 */
async function runGame(backend: BiwaBackend): Promise<void> {
  switch (backend.kind) {
    case "typescript": {
      // ゲームコードもこのスレッドで走るので、預けた関数はこのスレッドの表にある。
      ui.setHandlerReleaser(releaseHandlers);
      backend.entrypoint();

      // 2 回目以降の開始は未定義 (§19 の R8) なので、最初の 1 回だけ受け取る。
      const request = await new Promise<SceneStartRequest>((resolve) => {
        ui.setSceneStarter(resolve);
      });
      ui.setSceneStarter(() => {});
      const onClick = lookupHandler(request.onClick);
      const mainScene = lookupHandler(request.mainScene);
      if (onClick === undefined || mainScene === undefined) {
        throw new Error("[biwa] the handlers to start the scene are not retained");
      }

      const game = onClick(
        backend.gameWindowNew(request.canvasId, request.messageAreaId),
      );
      ui.enterScenePage(request.windowId);
      // scene は generator なので、呼んだだけでは何も起きない。
      // kernel が next() で駆動し、yield された syscall を処理して結果を書き戻す。
      // (TypeScript ではまだ scene を関数の値にできない (R2) ので、ここには届かない)
      const kernel = new Kernel(createSyscallTable());
      await kernel.run(mainScene(game) as BiwaScene);
      return;
    }
    case "wasm": {
      // 生成物は Worker で走る。ブロッキング syscall は Worker のスレッドを止める。
      // buildId を付けるのは、再ビルドで同じ URL のまま中身が変わるためである。
      const url = new URL(backend.url, location.href);
      url.searchParams.set("v", backend.buildId);
      await runWasm(url.href);
      return;
    }
  }
}
