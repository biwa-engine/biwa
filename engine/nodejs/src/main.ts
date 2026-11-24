import { setEngineContext } from "./engine/api/context";
import { CanvasObjects } from "./engine/canvas/CanvasObjects";
import { CanvasSurfaces } from "./engine/canvas/CanvasSurfaces";
import { Renderer } from "./engine/Renderer";
import { SoundEngine } from "./engine/sound/SoundEngine";
import { UIObjects } from "./engine/ui/UIObjects";
import { runWasm } from "./engine/vm/wasm/host";
import game from "./game/entry";
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
// 音は canvas と同じ時計で動き、sync 印の待ちにも加わる (`docs/sound.md`)。
const sound = new SoundEngine();
objects.addSyncPeer(sound.mixer);
// ブラウザは利用者の操作があるまで音を出させない。最初の操作で許してもらう。
for (const type of ["pointerdown", "keydown"]) {
  window.addEventListener(type, () => sound.resume(), { capture: true });
}
// エンジンが Ticker に登録するコールバックはこれ 1 つだけである。
// オブジェクトごとに生やさないのは、リークを避けるためでもあるし、
// ポーズ・オート・スキップを 1 箇所の時間操作で効かせるためでもある。
renderer.ticker.add((ticker) => {
  objects.update(ticker.deltaMS);
  sound.update(objects.clock);
  // 音が終わって sync の待ちが解けたかを、音の更新の後に見直す。
  objects.settle();
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
  sound,
  ui,
  host,
});

document.title = `${game.packageName} — Biwa`;

// エンジンは UI を何も置かない。`fn main()` が Window を表示して初めて画面に何かが出る。
//
// 流れ: `fn main()` で Window を表示 → SceneStartButton が押されるのを待つ →
// Window に預けてあった `WindowHandle` をボタンの `on_click(window)` に渡して `Game[S]` を作る →
// ScenePage を見せる → Window の `main_scene(game)`。
// 生成物は Worker で走り、ブロッキング syscall は Worker のスレッドを止める (`engine/vm/wasm/`)。
// buildId を付けるのは、再ビルドで同じ URL のまま中身が変わるためである。
const url = new URL(game.url, location.href);
url.searchParams.set("v", game.buildId);
await runWasm(url.href);
