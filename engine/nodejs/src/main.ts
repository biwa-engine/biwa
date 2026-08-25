import { ComponentRegistry } from "./components/ComponentRegistry";
import { TextBox } from "./components/TextBox";
import { setEngineContext } from "./engine/api/context";
import { createInitialGame } from "./engine/game";
import { Renderer } from "./engine/Renderer";
import { Kernel } from "./engine/vm/kernel";
import { createSyscallTable } from "./engine/vm/handlers";
import entrypoint, { packageName } from "./game/entry";

const WIDTH = 1280;
const HEIGHT = 720;

const MESSAGE_BOX_ID = "main-message-window";
const SPRITE_LAYER_ID = "chara";

const host = document.querySelector<HTMLDivElement>("#app")!;
host.style.cursor = "pointer";

const renderer = new Renderer(host);
await renderer.init(WIDTH, HEIGHT);

renderer.layers.defineLayer({ id: "background", type: "canvas", zIndex: 0 });
renderer.layers.defineLayer({ id: SPRITE_LAYER_ID, type: "canvas", zIndex: 10 });
renderer.layers.defineLayer({ id: "message", type: "dom", zIndex: 20 });

const components = new ComponentRegistry();
components.register(
  MESSAGE_BOX_ID,
  new TextBox(0, 460, WIDTH, 260),
  renderer.layers.getDom("message"),
);

// 以降、syscall の実装はこのコンテキストを通してエンジンを触る。
setEngineContext({
  renderer,
  components,
  host,
  messageBoxId: MESSAGE_BOX_ID,
  spriteLayerId: SPRITE_LAYER_ID,
});

document.title = `${packageName} — Biwa`;

// scene は generator なので、呼んだだけでは何も起きない。
// kernel が next() で駆動し、yield された syscall を処理して結果を書き戻す。
const kernel = new Kernel(createSyscallTable());
await kernel.run(entrypoint(createInitialGame(packageName)));
