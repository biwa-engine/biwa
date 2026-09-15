/**
 * Message Window の実体。
 *
 * 積まれた content は**断片の列**として持つ。1 本の文字列にしないのは、
 * 断片ごとに色・大きさ・太さ・速度が違いうるからである
 * (`docs/content-api.md` の Content API)。
 *
 * 描き始める時機は std が決める。`push()` は積むだけで何も起きず、
 * `flush()` で初めて DOM に入る。`clear()` も std からしか呼ばれない。
 */

/** 1 つの content。値はすべて解決済みの絶対値である。 */
export interface TextFragment {
  text: string;
  /** 秒間何文字出るか。文字送りは段 4 で入る。 */
  speed: number;
  /** 0 = vw, 1 = vh。 */
  sizeUnit: number;
  /** 単位に対する値。100 で viewport の 100 %。 */
  sizeValue: number;
  /** 100 〜 900。 */
  weight: number;
  /** 0 〜 255。 */
  r: number;
  g: number;
  b: number;
  a: number;
}

export class TextBox {
  readonly element: HTMLElement;
  private textEl: HTMLParagraphElement;

  /** まだ `flush()` されていない断片。 */
  private pending: TextFragment[] = [];

  constructor(x: number, y: number, width: number, height: number) {
    this.element = document.createElement("div");
    this.element.style.cssText = `
      position: absolute;
      left: ${x}px;
      top: ${y}px;
      width: ${width}px;
      height: ${height}px;
      background: rgba(0, 0, 0, 0.75);
      color: #fff;
      padding: 24px 32px;
      font-size: 18px;
      line-height: 1.8;
      box-sizing: border-box;
      overflow-y: auto;
      display: none;
    `;

    this.textEl = document.createElement("p");
    // ノベルテキストは改行や字下げを含んだまま渡ってくるので、そのまま見せる。
    this.textEl.style.cssText = "margin: 0; white-space: pre-wrap;";
    this.element.appendChild(this.textEl);
  }

  show(): void {
    this.element.style.display = "block";
  }

  hide(): void {
    this.element.style.display = "none";
  }

  /** 断片を 1 つ積む。まだ描かない。 */
  push(fragment: TextFragment): void {
    this.pending.push(fragment);
  }

  /**
   * 積まれた断片を描く。
   *
   * TODO(段 4): ここで文字送りのアニメーションを始める。
   * いまは一度に全部出している。
   */
  flush(): void {
    for (const fragment of this.pending) {
      this.textEl.appendChild(renderFragment(fragment));
    }
    this.pending = [];
  }

  /** 枠を空にする。積んだだけでまだ出していないものも捨てる。 */
  clear(): void {
    this.textEl.textContent = "";
    this.pending = [];
  }
}

/**
 * 断片を 1 つの `<span>` にする。
 *
 * 大きさは vw / vh をそのまま CSS の単位として使う。
 * biwa 側の `Size` がこの 2 つしか持たないのは、
 * ピクセルだと画面の大きさに追従できないからである。
 */
function renderFragment(fragment: TextFragment): HTMLSpanElement {
  const span = document.createElement("span");
  const unit = fragment.sizeUnit === 0 ? "vw" : "vh";

  span.textContent = fragment.text;
  span.style.fontSize = `${fragment.sizeValue}${unit}`;
  span.style.fontWeight = String(fragment.weight);
  span.style.color =
    `rgba(${fragment.r}, ${fragment.g}, ${fragment.b}, ${fragment.a / 255})`;

  return span;
}
