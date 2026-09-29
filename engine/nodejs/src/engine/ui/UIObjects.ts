/**
 * UI Element のツリーと、その DOM への反映。
 *
 * `CanvasObjects` (`engine/canvas/CanvasObjects.ts`) の UI 版にあたる。
 * biwa 側のパラメータを正とし、DOM は毎回の property 設定・push_child で
 * その場で更新する (canvas と違い、時間で変化する値を持たないので射影は要らない)。
 *
 * 設計は `docs/ui-api.md`、実装方針は `docs/ui-api-impl-status.md` にある。
 */

import { resolveAssetUrl } from "../api/assets";
import {
  ElementKind,
  isKnownElementKind,
  isKnownNumericProperty,
  isKnownStringProperty,
  PropertyKind,
  Unit,
} from "../api/ui";

/** UI Element 1 つ。 */
interface UiNode {
  readonly id: number;
  readonly kind: number;
  readonly dom: HTMLElement;
  parent: UiNode | null;
  children: UiNode[];
  /** Link のクリック時の遷移先 page_id。Link 以外では使わない。 */
  onClickLink: string | null;
  /** Page 自身の識別子。Page 以外では使わない。 */
  pageId: string | null;
}

/** 親が子をいくつ持てるか。`docs/ui-api.md` の push_child の規則そのもの。 */
type ChildCapacity = "many" | "single" | "none";

function childCapacity(kind: number): ChildCapacity {
  switch (kind) {
    case ElementKind.Window:
    case ElementKind.Page:
    case ElementKind.Horizontal:
    case ElementKind.Vertical:
    case ElementKind.HorizontalGrid:
      return "many";
    case ElementKind.Box:
      return "single";
    default:
      // Link や、まだ知らない kind はここに来る。子は持てない。
      return "none";
  }
}

/**
 * UI Element の管理。
 *
 * `ui_id → UiNode` を持ち、syscall のたびに対応する DOM を書き換える。
 * canvas と違って Ticker には登録しない (時間で動くパラメータが無いため)。
 */
export class UIObjects {
  private readonly root: HTMLElement;
  private readonly nodes = new Map<number, UiNode>();
  private nextId = 1;

  constructor(root: HTMLElement) {
    this.root = root;
  }

  /**
   * 新しい ui_id を採る。
   *
   * TypeScript ターゲット用。wasm ターゲットでは Worker が自分で採番する
   * (`CanvasObjects.allocId` と同じ理由)。
   */
  allocId(): number {
    return this.nextId++;
  }

  // --- syscall の実体 -----------------------------------------------------

  /**
   * UI Element を作る。
   *
   * `Window` だけは、それ自身を子として積む先が無いので、作った時点で
   * ルートの DOM 層に直接マウントする。他の kind は `pushChild` されるまで
   * どこにも繋がらない (create と push を分けて、property を設定し終えてから
   * アトミックに出現させるための設計 — `docs/ui-api.md`)。
   */
  create(id: number, kind: number): void {
    if (this.nodes.has(id)) {
      console.error(`[biwa] ui element ${id} already exists`);
      return;
    }
    if (!isKnownElementKind(kind)) {
      console.error(`[biwa] unknown ui element kind: ${kind}`);
      return;
    }

    const node: UiNode = {
      id,
      kind,
      dom: createDom(kind),
      parent: null,
      children: [],
      onClickLink: null,
      pageId: null,
    };
    this.nodes.set(id, node);

    if (kind === ElementKind.Window) {
      this.root.appendChild(node.dom);
    }
  }

  /** 数値の property を設定する。 */
  setProperty(
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
    const node = this.nodes.get(id);
    if (node === undefined) {
      console.error(`[biwa] no such ui element: ${id}`);
      return;
    }
    if (!isKnownNumericProperty(kind)) {
      console.error(`[biwa] unknown or non-numeric ui property: ${kind}`);
      return;
    }

    applyNumericProperty(
      node,
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
  setPropertyString(
    id: number,
    kind: number,
    valU: number,
    valI: number,
    valF: number,
    valS: string,
  ): void {
    const node = this.nodes.get(id);
    if (node === undefined) {
      console.error(`[biwa] no such ui element: ${id}`);
      return;
    }
    if (!isKnownStringProperty(kind)) {
      console.error(`[biwa] unknown or non-string ui property: ${kind}`);
      return;
    }

    this.applyStringProperty(node, kind, valU, valI, valF, valS);
  }

  /**
   * 子 Element を親に積む。
   *
   * 親が子を
   * - 複数持てるなら末尾に追加
   * - 1 つのみ持てるなら、無ければ追加・あれば既存を削除して置き換え
   * - 持てないなら、子はツリーから削除される (見た目上は何も起こらない)
   */
  pushChild(parentId: number, childId: number): void {
    const parent = this.nodes.get(parentId);
    const child = this.nodes.get(childId);
    if (parent === undefined) {
      console.error(`[biwa] no such ui element (parent): ${parentId}`);
      return;
    }
    if (child === undefined) {
      console.error(`[biwa] no such ui element (child): ${childId}`);
      return;
    }

    switch (childCapacity(parent.kind)) {
      case "none":
        this.destroy(child);
        return;
      case "single":
        for (const existing of [...parent.children]) {
          this.destroy(existing);
        }
        this.attach(parent, child);
        return;
      case "many":
        this.attach(parent, child);
        return;
    }
  }

  // --- 内部 -----------------------------------------------------------------

  private attach(parent: UiNode, child: UiNode): void {
    if (child.parent !== null) {
      this.detachFromParent(child);
    }
    parent.children.push(child);
    child.parent = parent;
    parent.dom.appendChild(child.dom);

    if (child.kind === ElementKind.Page) {
      this.showFirstPageIfNoneVisible(parent, child);
    }
  }

  private detachFromParent(node: UiNode): void {
    const parent = node.parent;
    if (parent === null) return;
    parent.children = parent.children.filter((c) => c !== node);
    node.dom.remove();
    node.parent = null;
  }

  /** ツリーから消す。子も道連れにする。 */
  private destroy(node: UiNode): void {
    this.detachFromParent(node);
    for (const child of [...node.children]) {
      this.destroy(child);
    }
    this.nodes.delete(node.id);
  }

  private applyStringProperty(
    node: UiNode,
    kind: number,
    _valU: number,
    _valI: number,
    _valF: number,
    valS: string,
  ): void {
    switch (kind) {
      case PropertyKind.PageId:
        node.pageId = valS;
        return;
      case PropertyKind.OnClickLink:
        node.onClickLink = valS;
        this.ensureLinkClickHandler(node);
        return;
      case PropertyKind.Text:
        node.dom.textContent = valS;
        return;
      case PropertyKind.TextFont:
        node.dom.style.fontFamily = valS;
        return;
      case PropertyKind.BackgroundImage:
        node.dom.style.backgroundColor = "";
        node.dom.style.backgroundImage = `url(${resolveAssetUrl(valS)})`;
        node.dom.style.backgroundSize = "cover";
        node.dom.style.backgroundPosition = "center";
        return;
      case PropertyKind.Image:
        if (node.kind !== ElementKind.Image) {
          console.error(
            `[biwa] "image" property is only meaningful on Image (ui element ${node.id})`,
          );
          return;
        }
        node.dom.style.backgroundImage = `url(${resolveAssetUrl(valS)})`;
        return;
      default:
        console.error(`[biwa] unhandled string ui property: ${kind}`);
    }
  }

  /**
   * Link の click ハンドラを 1 度だけ張る。
   *
   * クリックは host まで伝播させない。伝播させると Message Window の
   * クリック待ち (`waitForClick`) も同時に反応してしまう。
   */
  private ensureLinkClickHandler(node: UiNode): void {
    if (node.dom.dataset["biwaLinkBound"] === "1") return;
    node.dom.dataset["biwaLinkBound"] = "1";
    node.dom.addEventListener("click", (event) => {
      event.stopPropagation();
      this.handleLinkClick(node);
    });
  }

  private handleLinkClick(link: UiNode): void {
    const target = link.onClickLink;
    if (target === null) return;

    const win = ownerWindow(link);
    if (win === null) {
      console.error("[biwa] a Link outside of any Window was clicked");
      return;
    }
    this.showPage(win, target);
  }

  /** Window 直下にまだ見えている Page が無ければ、追加された Page を見せる。 */
  private showFirstPageIfNoneVisible(window: UiNode, page: UiNode): void {
    const alreadyVisible = window.children.some(
      (c) => c.kind === ElementKind.Page && c !== page && isVisible(c),
    );
    if (!alreadyVisible) {
      page.dom.style.display = "";
    }
  }

  /** Window 直下の Page を、page_id が一致するものだけ見せる。 */
  private showPage(window: UiNode, pageId: string): void {
    let found = false;
    for (const child of window.children) {
      if (child.kind !== ElementKind.Page) continue;
      const match = child.pageId === pageId;
      child.dom.style.display = match ? "" : "none";
      found ||= match;
    }
    if (!found) {
      console.error(`[biwa] no such page in this window: "${pageId}"`);
    }
  }
}

/** `kind` に対応する DOM を作る。中身は空で、property・child は後から付く。 */
function createDom(kind: number): HTMLElement {
  switch (kind) {
    case ElementKind.Window:
      return styled(document.createElement("div"), {
        position: "absolute",
        inset: "0",
        // Window 自身はクリックを奪わない。奪うのは Link だけ。
        pointerEvents: "none",
      });
    case ElementKind.Page:
      return styled(document.createElement("div"), {
        position: "absolute",
        inset: "0",
        // 最初は隠れている。表示は `showFirstPageIfNoneVisible` / `showPage` が決める。
        display: "none",
      });
    case ElementKind.Box:
      return styled(document.createElement("div"), {
        position: "relative",
      });
    case ElementKind.Link:
      return styled(document.createElement("div"), {
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        textAlign: "center",
        cursor: "pointer",
        pointerEvents: "auto",
      });
    case ElementKind.Horizontal:
      return styled(document.createElement("div"), {
        display: "flex",
        flexDirection: "row",
      });
    case ElementKind.Vertical:
      return styled(document.createElement("div"), {
        display: "flex",
        flexDirection: "column",
      });
    case ElementKind.HorizontalGrid:
      return styled(document.createElement("div"), {
        display: "grid",
        gridTemplateColumns: "repeat(1, 1fr)",
      });
    case ElementKind.Image:
      return styled(document.createElement("div"), {
        backgroundSize: "cover",
        backgroundPosition: "center",
        backgroundRepeat: "no-repeat",
      });
    default:
      // `isKnownElementKind` を先に通しているので、ここには来ない想定。
      throw new Error(`[biwa] unknown ui element kind: ${kind}`);
  }
}

function styled(
  el: HTMLElement,
  style: Partial<CSSStyleDeclaration>,
): HTMLElement {
  Object.assign(el.style, style);
  return el;
}

function applyNumericProperty(
  node: UiNode,
  kind: number,
  valU1: number,
  valU2: number,
  valU3: number,
  valU4: number,
  valI1: number,
  valI2: number,
  valF: number,
): void {
  switch (kind) {
    case PropertyKind.Width:
      node.dom.style.width = sizeToCss(valU1, valF);
      return;
    case PropertyKind.Height:
      node.dom.style.height = sizeToCss(valU1, valF);
      return;
    case PropertyKind.MarginLeft:
      node.dom.style.marginLeft = sizeToCss(valU1, valF);
      return;
    case PropertyKind.MarginRight:
      node.dom.style.marginRight = sizeToCss(valU1, valF);
      return;
    case PropertyKind.MarginTop:
      node.dom.style.marginTop = sizeToCss(valU1, valF);
      return;
    case PropertyKind.MarginBottom:
      node.dom.style.marginBottom = sizeToCss(valU1, valF);
      return;
    case PropertyKind.PaddingLeft:
      node.dom.style.paddingLeft = sizeToCss(valU1, valF);
      return;
    case PropertyKind.PaddingRight:
      node.dom.style.paddingRight = sizeToCss(valU1, valF);
      return;
    case PropertyKind.PaddingTop:
      node.dom.style.paddingTop = sizeToCss(valU1, valF);
      return;
    case PropertyKind.PaddingBottom:
      node.dom.style.paddingBottom = sizeToCss(valU1, valF);
      return;
    case PropertyKind.BackgroundColor:
      node.dom.style.backgroundImage = "";
      node.dom.style.backgroundColor = `rgba(${valU1}, ${valU2}, ${valU3}, ${valU4 / 255})`;
      return;
    case PropertyKind.Column:
      if (node.kind !== ElementKind.HorizontalGrid) {
        console.error(
          `[biwa] "column" property is only meaningful on HorizontalGrid (ui element ${node.id})`,
        );
        return;
      }
      node.dom.style.gridTemplateColumns = `repeat(${Math.max(valU1, 1)}, 1fr)`;
      return;
    case PropertyKind.TextSize:
      node.dom.style.fontSize = sizeToCss(valU1, valF);
      return;
    case PropertyKind.TextWeight:
      node.dom.style.fontWeight = String(valU1);
      return;
    case PropertyKind.TextColor:
      node.dom.style.color = `rgba(${valU1}, ${valU2}, ${valU3}, ${valU4 / 255})`;
      return;
    default:
      // `isKnownNumericProperty` を先に通しているので、ここには来ない想定。
      console.error(`[biwa] unhandled numeric ui property: ${kind}`);
  }

  // unused: val_i1 / val_i2 は現行の数値 property (すべて非負) では使わないが、
  // 符号付きの位置指定などを足すときのために syscall のシグネチャには残してある。
  void valI1;
  void valI2;
}

function sizeToCss(unit: number, value: number): string {
  switch (unit) {
    case Unit.Vw:
      return `${value}vw`;
    case Unit.Vh:
      return `${value}vh`;
    case Unit.Percent:
      return `${value}%`;
    default:
      console.error(`[biwa] unknown ui size unit: ${unit}`);
      return "0";
  }
}

function isVisible(node: UiNode): boolean {
  return node.dom.style.display !== "none";
}

/** 祖先を辿って、自分を含む最も近い Window を探す。 */
function ownerWindow(node: UiNode): UiNode | null {
  let cur: UiNode | null = node;
  while (cur !== null) {
    if (cur.kind === ElementKind.Window) return cur;
    cur = cur.parent;
  }
  return null;
}
