import { ComponentRegistry } from "./components/ComponentRegistry";
import { TextBox } from "./components/TextBox";
import { setEngineContext } from "./engine/api/context";
import { createInitialGame } from "./engine/game";
import { Renderer } from "./engine/Renderer";
import entrypoint, { packageName } from "./game/entry";

const WIDTH = 1280;
const HEIGHT = 720;

const MESSAGE_BOX_ID = "main-message-window";
const SPRITE_LAYER_ID = "chara";

const host = document.querySelector<HTMLDivElement>("#app")!;

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

// 以降、生成されたゲームコードは std 経由でこのコンテキストを触る。
setEngineContext({
  renderer,
  components,
  messageBoxId: MESSAGE_BOX_ID,
  spriteLayerId: SPRITE_LAYER_ID,
});

document.title = `${packageName} — Biwa`;

// ゲーム本体。現状は同期関数なので、ここで scene main が最後まで走りきる。
entrypoint(createInitialGame(packageName));
