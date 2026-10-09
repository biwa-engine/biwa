/**
 * UI syscall の kind 番号と、その実装への合流点。
 *
 * Element kind / property kind の番号は std (`library/std/src/game/base_engine.biwa`)
 * との唯一の合意点である。wasm 側の std はこれを `.wat` に直書きするので、
 * 写し間違いは `UIObjects` が名指しで叱る (`isKnownElementKind` などを経由する)。
 * この番号表自体は `Param` / `Curve` (`api/transition.ts`) と同じ役割である。
 *
 * 設計は `docs/ui-api.md`、実装方針は `docs/ui-api-impl-status.md` にある。
 */

import { engine } from "./context";

/** UI Element の種類。 */
export const ElementKind = {
  Window: 0,
  Page: 1,
  Box: 2,
  Link: 3,
  Horizontal: 4,
  Vertical: 5,
  HorizontalGrid: 6,
  /** std 側の型名は `UiImage` (`std::game::image::Image` との衝突を避けるため)。 */
  Image: 7,
  /**
   * Canvas API の出力先。std の `GameCanvas` がこの ui_id を持つ。
   * 子は持てない。
   */
  Canvas: 8,
  /**
   * Content API (ノベル表現) の出力先。Message Window (`TextBox`) を 1 つ持つ。
   * std の `GameMessageArea` がこの ui_id を持つ。子は持てない。
   */
  MessageArea: 9,
  /**
   * 子を重ねる Element。子は Layers の親の中にいるのと同じように配置され
   * (x / y 方向には互いに干渉しない)、push された順に上に積み上がる。
   */
  Layers: 10,
  /**
   * scene を映すページ。Window が 1 つだけ持つ (`WindowScenePage` property で結びつける)。
   * Link では遷移できず、scene が始まるときに表示される。
   * 子は Page と同じく自由に持てる。scene の出力先は `ScenePageCanvas` / `ScenePageMessageArea`
   * で指し、その実体は子孫に置かれていなければならない (Window に結びつけるときに確かめる)。
   */
  ScenePage: 11,
  /**
   * 押すと scene を始めるボタン。見た目の property は Link と同じ。子は持てない。
   * `SceneStartButtonOnClick` のハンドラを持つ。
   */
  SceneStartButton: 12,
  // Button はまだ実装しない (`docs/ui-api.md`)。
} as const;

const KNOWN_ELEMENT_KINDS = new Set<number>(Object.values(ElementKind));

export function isKnownElementKind(kind: number): boolean {
  return KNOWN_ELEMENT_KINDS.has(kind);
}

/**
 * UI property の種類。
 *
 * `sys_ui_set_property` (数値だけ) と `sys_ui_set_property_with_string`
 * (文字列を伴う) のどちらで運ばれるかは kind ごとに決まっている。
 * 番号の帯を分けてあるのは、取り違えを早く見つけるための整理でしかなく、
 * syscall 自体はそれを見て振り分けているわけではない。
 */
export const PropertyKind = {
  // --- sys_ui_set_property (数値: val_u1..4 / val_i1..2 / val_f) ---
  /** unit: val_u1, value: val_f。 */
  Width: 0,
  /** unit: val_u1, value: val_f。 */
  Height: 1,
  /** unit: val_u1, value: val_f。 */
  MarginLeft: 2,
  MarginRight: 3,
  MarginTop: 4,
  MarginBottom: 5,
  PaddingLeft: 6,
  PaddingRight: 7,
  PaddingTop: 8,
  PaddingBottom: 9,
  /** r, g, b, a (0〜255) を val_u1..val_u4 に積む。1 語にパックしない (`docs/media-syscall-wasm.md` と同じ理由)。 */
  BackgroundColor: 10,
  /** HorizontalGrid の列数。val_u1 だけを使う。 */
  Column: 11,
  /** unit: val_u1, value: val_f。 */
  TextSize: 12,
  /** 100〜900。val_u1 だけを使う (Content API の TextWeight と同じ規約)。 */
  TextWeight: 13,
  /** r, g, b, a (0〜255) を val_u1..val_u4 に積む。BackgroundColor と同じ理由。 */
  TextColor: 14,
  /**
   * Window の ScenePage。val_u1 に ScenePage の ui_id を積む。Window 以外には付けられない。
   * 設定すると ScenePage はその Window の (隠れた) 子になる。
   */
  WindowScenePage: 15,
  /**
   * ScenePage の scene の出力先。val_u1 に Canvas / MessageArea の ui_id を積む。ScenePage 以外には付けられない。
   * 指す Element は ScenePage の子孫でなければならない。
   */
  ScenePageCanvas: 16,
  ScenePageMessageArea: 17,

  // --- sys_ui_set_property_with_string (文字列: val_s) ---
  /** Page が持つ識別子。Link の遷移先として参照される。 */
  PageId: 100,
  /** Link のクリック時の遷移先 page_id。 */
  OnClickLink: 101,
  /** Link / Box に表示する文字列。 */
  Text: 102,
  /** CSS の font-family としてそのまま使うフォント名。 */
  TextFont: 103,
  /** 背景画像のパス (`assets/` 基準)。BackgroundColor とは排他 (std 側で保証する)。 */
  BackgroundImage: 104,
  /** `Image` Element が表示する画像のパス (`assets/` 基準)。 */
  Image: 105,
  /**
   * すべての Element が持てる任意の識別子。
   *
   * `docs/ui-api.md` の Page の `canvas`/`message_area` property (Step3) や、
   * ホストから ui_id を引く仕組みの土台になる。id からその Element の
   * ui_id を引けるようにするのがエンジン側の `UIObjects` の役目である。
   */
  Id: 106,
  // 107〜109 は欠番 (以前は文字列の id で scene の出力先と scene の Page を指していた。今は ScenePage)。
} as const;

const NUMERIC_PROPERTY_KINDS = new Set<number>([
  PropertyKind.Width,
  PropertyKind.Height,
  PropertyKind.MarginLeft,
  PropertyKind.MarginRight,
  PropertyKind.MarginTop,
  PropertyKind.MarginBottom,
  PropertyKind.PaddingLeft,
  PropertyKind.PaddingRight,
  PropertyKind.PaddingTop,
  PropertyKind.PaddingBottom,
  PropertyKind.BackgroundColor,
  PropertyKind.Column,
  PropertyKind.TextSize,
  PropertyKind.TextWeight,
  PropertyKind.TextColor,
  PropertyKind.WindowScenePage,
  PropertyKind.ScenePageCanvas,
  PropertyKind.ScenePageMessageArea,
]);

const STRING_PROPERTY_KINDS = new Set<number>([
  PropertyKind.PageId,
  PropertyKind.OnClickLink,
  PropertyKind.Text,
  PropertyKind.TextFont,
  PropertyKind.BackgroundImage,
  PropertyKind.Image,
  PropertyKind.Id,
]);

export function isKnownNumericProperty(kind: number): boolean {
  return NUMERIC_PROPERTY_KINDS.has(kind);
}

export function isKnownStringProperty(kind: number): boolean {
  return STRING_PROPERTY_KINDS.has(kind);
}

/**
 * ハンドラ (Biwa から預かってホストが後で呼ぶ関数) の種類。
 *
 * `sys_ui_set_handler` で Element に設定する。ホストは種類ごとに決まった形
 * (引数と戻り値) で呼ぶ。形が合っていることは std の型付きの API が保証し、
 * ホストは引数・戻り値の中身を見ない (`docs/host-function-values.md`)。
 *
 * 呼ぶのは SceneStartButton が押されたとき (`UIObjects` の `SceneStartRequest` → wasm は Worker、
 * TypeScript は `main.ts`)。`on_click` で `Game[S]` を作り、ScenePage を表示してから `main_scene` を呼ぶ。
 */
export const HandlerKind = {
  /**
   * Window の `main_scene` (R4)。`Scene[S]` = `fn(Game[S]) -> Game[S]`。
   * Window 以外には付けられない。
   */
  WindowMainScene: 0,
  /** SceneStartButton の `on_click` (R6)。`fn(GameWindow) -> Game[S]`。 */
  SceneStartButtonOnClick: 1,
} as const;

/** ハンドラの種類 → それを付けられる Element の kind。 */
const HANDLER_TARGETS = new Map<number, number>([
  [HandlerKind.WindowMainScene, ElementKind.Window],
  [HandlerKind.SceneStartButtonOnClick, ElementKind.SceneStartButton],
]);

export function isKnownHandlerKind(kind: number): boolean {
  return HANDLER_TARGETS.has(kind);
}

/** その種類のハンドラを付けられる Element の kind。知らない種類なら `null`。 */
export function handlerTarget(kind: number): number | null {
  return HANDLER_TARGETS.get(kind) ?? null;
}

/**
 * `width` / `margin` などが取れる単位。
 *
 * Message Window の `Size` (`docs/content-api.md`) が vw/vh の 2 つしか
 * 持たないのと違い、UI のレイアウトでは `percent` (親要素に対する割合) も要る。
 */
export const Unit = {
  Vw: 0,
  Vh: 1,
  Percent: 2,
} as const;

// --- syscall の実装 (engine().ui への合流) ----------------------------------
//
// どのターゲットから来てもここに合流する (`api/object.ts` と同じ構造)。
// 中断しない syscall なので、失敗してもログに出すに留める
// (呼び出し元は積んで直ちに返っているため、投げても届かない)。

/** 新しい ui_id を採る。TypeScript ターゲット用 (wasm は Worker が自分で採る)。 */
export function allocUiId(): number {
  return engine().ui.allocId();
}

/** UI Element を作る。 */
export function createUiElement(id: number, kind: number): void {
  engine().ui.create(id, kind);
}

/** 数値の property を設定する。 */
export function setUiProperty(
  id: number,
  kind: number,
  valU1: number,
  valU2: number,
  valU3: number,
  valU4: number,
  valI1: number,
  valI2: number,
  valF: number,
): void {
  engine().ui.setProperty(
    id,
    kind,
    valU1,
    valU2,
    valU3,
    valU4,
    valI1,
    valI2,
    valF,
  );
}

/** 文字列を伴う property を設定する。 */
export function setUiPropertyString(
  id: number,
  kind: number,
  valU: number,
  valI: number,
  valF: number,
  valS: string,
): void {
  engine().ui.setPropertyString(id, kind, valU, valI, valF, valS);
}

/**
 * 子 Element を親に積む。
 *
 * 親が子を複数持てる場合は末尾に追加、1つのみ持てる場合は既存を置き換え、
 * 持てない場合は子が Element リストから削除される (`docs/ui-api.md` の Syscall 節)。
 */
export function pushUiChild(parent: number, child: number): void {
  engine().ui.pushChild(parent, child);
}

/**
 * Element にハンドラを設定する。`handle` は預けた関数の番号である
 * (wasm は Worker の表、TypeScript は `api/handler.ts` の表)。
 *
 * 同じ Element・同じ種類に設定し直すと置き換わり、古い関数は手放される。
 */
export function setUiHandler(id: number, kind: number, handle: number): void {
  engine().ui.setHandler(id, kind, handle);
}
