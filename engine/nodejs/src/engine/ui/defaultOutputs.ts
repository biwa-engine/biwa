/**
 * 既定の出力先 (Canvas / MessageArea)。**暫定**である。
 *
 * `on_new_game(window: GameWindow)` には出力先の ui_id を詰めた `GameWindow` を渡す。
 * 本来はゲーム側の `fn app() -> Window` が置いた `<Canvas id>` / `<MessageArea id>` を
 * scene 用 Page の property から引く (`docs/ui-api-impl-status.md` §14 S7/S8)。
 * それができるまでは `on_new_game` の時点でゲーム側の UI の木がまだ無いので、
 * エンジンが起動時に既定の Window → Page → (Canvas, MessageArea) を作り、その ui_id を渡す。
 * S8 (`app()`) でこのファイルごと消す。
 *
 * 見た目は、以前の固定の canvas (画面全体) と Message Window
 * (`left: 0; top: 460px; 1280x260`、`rgba(0,0,0,0.75)`、`padding: 24px 32px`) を
 * UI Element の property で再現している。
 *
 * - Canvas は画面全体を覆う (Canvas API の描画範囲は Element の矩形なので、
 *   背景をメッセージ枠の裏まで描くにはこうする必要がある)。
 * - MessageArea はそれに重ねたいが、位置指定の property が無い。そこで Window を 2 つ作り
 *   (Window はどれも画面全体に重なる)、2 つ目の Page の中で
 *   高さ 460/720 の空の Box の下に MessageArea (260/720) を積む。
 * - padding の `%` は CSS の規則どおり親の**幅**に対する割合 (24/1280, 32/1280) である。
 */

import { ElementKind, PropertyKind, Unit } from "../api/ui";
import type { UIObjects } from "./UIObjects";

/** 既定の出力先の ui_id。 */
export interface DefaultOutputs {
  canvasId: number;
  messageAreaId: number;
}

/** 以前の固定枠を描いていた画面の大きさ (px)。割合の計算にだけ使う。 */
const SCREEN_WIDTH = 1280;
const SCREEN_HEIGHT = 720;
const MESSAGE_AREA_HEIGHT = 260;

/**
 * 既定の出力先を作って表示し、その ui_id を返す。
 *
 * ゲーム側の Element より先に作るので、ゲームが後から出す Window はこれより前面に来る。
 */
export function createDefaultOutputs(ui: UIObjects): DefaultOutputs {
  const percent = (px: number, of: number): number => (px / of) * 100;

  // 1 つ目の Window: 画面全体の Canvas。
  const canvasWindowId = create(ui, ElementKind.Window);
  const canvasPageId = create(ui, ElementKind.Page);
  const canvasId = create(ui, ElementKind.Canvas);
  setSize(ui, canvasId, PropertyKind.Width, 100);
  setSize(ui, canvasId, PropertyKind.Height, 100);

  // 2 つ目の Window: 下端の MessageArea。Canvas の上に重なる。
  const messageWindowId = create(ui, ElementKind.Window);
  const messagePageId = create(ui, ElementKind.Page);
  const spacerId = create(ui, ElementKind.Box);
  const messageAreaId = create(ui, ElementKind.MessageArea);

  setSize(ui, spacerId, PropertyKind.Width, 100);
  setSize(
    ui,
    spacerId,
    PropertyKind.Height,
    percent(SCREEN_HEIGHT - MESSAGE_AREA_HEIGHT, SCREEN_HEIGHT),
  );

  setSize(ui, messageAreaId, PropertyKind.Width, 100);
  setSize(
    ui,
    messageAreaId,
    PropertyKind.Height,
    percent(MESSAGE_AREA_HEIGHT, SCREEN_HEIGHT),
  );
  setSize(ui, messageAreaId, PropertyKind.PaddingTop, percent(24, SCREEN_WIDTH));
  setSize(ui, messageAreaId, PropertyKind.PaddingBottom, percent(24, SCREEN_WIDTH));
  setSize(ui, messageAreaId, PropertyKind.PaddingLeft, percent(32, SCREEN_WIDTH));
  setSize(ui, messageAreaId, PropertyKind.PaddingRight, percent(32, SCREEN_WIDTH));
  // rgba(0, 0, 0, 0.75)。a は 0〜255。
  ui.setProperty(
    messageAreaId,
    PropertyKind.BackgroundColor,
    0,
    0,
    0,
    Math.round(0.75 * 255),
    0,
    0,
    0,
  );

  // create → property → push_child の順で、組み上がってから出現させる。
  ui.pushChild(canvasPageId, canvasId);
  ui.pushChild(canvasWindowId, canvasPageId);
  ui.pushChild(messagePageId, spacerId);
  ui.pushChild(messagePageId, messageAreaId);
  ui.pushChild(messageWindowId, messagePageId);

  return { canvasId, messageAreaId };
}

function create(ui: UIObjects, kind: number): number {
  const id = ui.allocId();
  ui.create(id, kind);
  return id;
}

function setSize(ui: UIObjects, id: number, kind: number, value: number): void {
  ui.setProperty(id, kind, Unit.Percent, 0, 0, 0, 0, 0, value);
}
