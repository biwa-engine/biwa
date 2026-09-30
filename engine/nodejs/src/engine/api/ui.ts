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
  // Button は funcref 前提のため実装しない (`docs/ui-api.md`)。
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

  // --- sys_ui_set_property_with_string (文字列: val_s) ---
  /** Page が持つ識別子。Link の遷移先として参照される。 */
  PageId: 100,
  /** Link のクリック時の遷移先 page_id。 */
  OnClickLink: 101,
  /** Link に表示する文字列。 */
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
