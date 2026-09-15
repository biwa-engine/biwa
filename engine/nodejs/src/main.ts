import { ComponentRegistry } from "./components/ComponentRegistry";
import { TextBox } from "./components/TextBox";
import { setEngineContext } from "./engine/api/context";
import { CanvasObjects } from "./engine/canvas/CanvasObjects";
import type { BiwaBackend } from "./engine/game";
import { Renderer } from "./engine/Renderer";
import { Kernel } from "./engine/vm/kernel";
import { createSyscallTable } from "./engine/vm/handlers";
import { runWasm } from "./engine/vm/wasm/host";
import backend from "./game/entry";

const WIDTH = 1280;
const HEIGHT = 720;

const MESSAGE_BOX_ID = "main-message-window";
const MESSAGE_LAYER_ID = "message";

const host = document.querySelector<HTMLDivElement>("#app")!;
host.style.cursor = "pointer";

const renderer = new Renderer(host);
await renderer.init(WIDTH, HEIGHT);

// canvas レイヤーは `create_object` の layer index から必要に応じて作られる。
// ここで定義するのは DOM レイヤーだけでよい。
renderer.layers.defineDom(MESSAGE_LAYER_ID, 20);

const components = new ComponentRegistry();
components.register(
  MESSAGE_BOX_ID,
  new TextBox(0, 460, WIDTH, 260),
  renderer.layers.dom(MESSAGE_LAYER_ID),
);

const objects = new CanvasObjects(renderer.layers, WIDTH, HEIGHT);
// エンジンが Ticker に登録するコールバックはこれ 1 つだけである。
// オブジェクトごとに生やさないのは、リークを避けるためでもあるし、
// ポーズ・オート・スキップを 1 箇所の時間操作で効かせるためでもある。
renderer.app.ticker.add((ticker) => {
  objects.update(ticker.deltaMS);
});

// 以降、syscall の実装はこのコンテキストを通してエンジンを触る。
setEngineContext({
  renderer,
  components,
  objects,
  host,
  messageBoxId: MESSAGE_BOX_ID,
});

document.title = `${backend.packageName} — Biwa`;

await runGame(backend);

/**
 * ゲームを走らせる。
 *
 * どちらのターゲットでも、エンジンから見えるのは syscall の流れだけである。
 * 違うのは「どこで動いていて、どうやって中断するか」でしかない。
 */
async function runGame(backend: BiwaBackend): Promise<void> {
  switch (backend.kind) {
    case "typescript": {
      // scene は generator なので、呼んだだけでは何も起きない。
      // kernel が next() で駆動し、yield された syscall を処理して結果を書き戻す。
      const kernel = new Kernel(createSyscallTable());
      await kernel.run(backend.entrypoint(backend.onNewGame()));
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
