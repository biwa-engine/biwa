/**
 * Message Window の実体。UI Element `MessageArea` が 1 つずつ持つ
 * (`engine/ui/UIObjects.ts`)。Content API はその ui_id で出力先を選ぶ。
 *
 * 積まれた content は**断片の列**として持つ。1 本の文字列にしないのは、
 * 断片ごとに色・大きさ・太さ・速度が違いうるからである
 * (`docs/content-api.md` の Content API)。
 *
 * 描き始める時機は std が決める。`push()` は積むだけで何も起きず、
 * `flush()` で初めて文字送りが始まる。`clear()` も std からしか呼ばれない。
 */

/** 1 つの content。値はすべて解決済みの絶対値である。 */
export interface TextFragment {
  text: string;
  /** 秒間何文字出るか。0 以下なら送らずに一度に出す。 */
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

/**
 * DOM に入った断片。
 *
 * 文字は最初から全部入れておき、まだ出ていない分を
 * `visibility: hidden` で隠す。`textContent` を伸ばしていく形にすると、
 * 1 文字進むたびに折り返しが変わって行がずれる。
 */
interface RevealedFragment {
  text: string;
  speed: number;
  /** 見えている文字数。 */
  revealed: number;
  shown: Text;
  hidden: Text;
}

export class TextBox {
  readonly element: HTMLElement;
  private textEl: HTMLParagraphElement;

  /** まだ `flush()` されていない断片。 */
  private pending: TextFragment[] = [];

  /** DOM に入っている断片。 */
  private revealing: RevealedFragment[] = [];

  /**
   * 文字送りのカーソル。
   *
   * 断片ごとに速度が違うので、「経過時間」では何文字目かを決められない。
   * 列に対する 1 本のカーソルと、1 文字に満たない端数 (`carry`) で持つ。
   */
  private cursor = 0;
  private carry = 0;

  /**
   * 親 (UI Element `MessageArea` の DOM) の内側 (padding の内側) を埋める。
   *
   * 位置・大きさ・背景・余白は `MessageArea` の property (width / padding /
   * background_color ...) が決める。TextBox 自身は枠の見た目を持たない。
   * 絶対配置にしないのは、親の padding を効かせるためである。
   */
  constructor() {
    this.element = document.createElement("div");
    this.element.style.cssText = `
      width: 100%;
      height: 100%;
      color: #fff;
      font-size: 18px;
      line-height: 1.8;
      box-sizing: border-box;
      overflow-y: auto;
    `;

    this.textEl = document.createElement("p");
    // ノベルテキストは改行や字下げを含んだまま渡ってくるので、そのまま見せる。
    this.textEl.style.cssText = "margin: 0; white-space: pre-wrap;";
    this.element.appendChild(this.textEl);
  }

  /** 断片を 1 つ積む。まだ描かない。 */
  push(fragment: TextFragment): void {
    this.pending.push(fragment);
  }

  /**
   * 積まれた断片を DOM に入れ、文字送りを始める。
   *
   * カーソルは触らない。前の `flush()` の送りが終わっていなければ、
   * 続きとしてそのまま繋がる。
   */
  flush(): void {
    for (const fragment of this.pending) {
      this.revealing.push(mount(this.textEl, fragment));
    }
    this.pending = [];
  }

  /** 枠を空にする。積んだだけでまだ出していないものも捨てる。 */
  clear(): void {
    this.textEl.textContent = "";
    this.pending = [];
    this.revealing = [];
    this.cursor = 0;
    this.carry = 0;
  }

  /**
   * 残りを全部出す。出すものがあったかを返す。
   *
   * クリックが来たときにまずこれを試す。飛ばすものがあれば、
   * そのクリックは送りの完了に使われてテキストは進まない
   * (canvas の `CanvasObjects.skipSync()` と同じ形)。
   */
  skip(): boolean {
    if (this.remaining() === 0) {
      this.cursor = this.revealing.length;
      return false;
    }

    for (let i = this.cursor; i < this.revealing.length; i += 1) {
      const fragment = this.revealing[i];
      reveal(fragment, fragment.text.length);
    }
    this.cursor = this.revealing.length;
    this.carry = 0;

    return true;
  }

  // --- Ticker から毎フレーム呼ばれる -----------------------------------

  /**
   * 文字送りを進める。
   *
   * `deltaMs` はエンジン時計の差分である (ポーズ・早送りを掛けたもの)。
   * `performance.now()` を見ないのは、時間まわりの操作を
   * 1 箇所に集めておくためである。
   */
  update(deltaMs: number): void {
    let seconds = deltaMs / 1000;

    while (this.cursor < this.revealing.length) {
      const fragment = this.revealing[this.cursor];
      const remaining = fragment.text.length - fragment.revealed;

      // 空の断片、出し切った断片、送らない断片は時間を使わない。
      // 時間の有無を見る前に片付けるので、速度 0 の断片は
      // 前の断片を出し切ったその場で出る。
      if (remaining === 0) {
        this.cursor += 1;
        continue;
      }
      if (fragment.speed <= 0) {
        reveal(fragment, fragment.text.length);
        this.cursor += 1;
        continue;
      }

      if (seconds <= 0) break;

      this.carry += seconds * fragment.speed;
      const step = Math.min(remaining, Math.floor(this.carry));
      this.carry -= step;
      reveal(fragment, fragment.revealed + step);

      if (step < remaining) {
        // まだ途中。このフレームの時間は使い切った。
        seconds = 0;
      } else {
        // 出し切った。余った端数を次の断片の時間として戻す。
        // 速度が断片ごとに違うので、文字数のままでは持ち越せない。
        seconds = this.carry / fragment.speed;
        this.carry = 0;
        this.cursor += 1;
      }
    }
  }

  /** まだ出ていない文字数。 */
  private remaining(): number {
    let count = 0;
    for (let i = this.cursor; i < this.revealing.length; i += 1) {
      count += this.revealing[i].text.length - this.revealing[i].revealed;
    }
    return count;
  }
}

/**
 * 断片を 1 つの `<span>` として DOM に入れる。
 *
 * 大きさは vw / vh をそのまま CSS の単位として使う。
 * biwa 側の `Size` がこの 2 つしか持たないのは、
 * ピクセルだと画面の大きさに追従できないからである。
 */
function mount(parent: HTMLElement, fragment: TextFragment): RevealedFragment {
  const span = document.createElement("span");
  const unit = fragment.sizeUnit === 0 ? "vw" : "vh";

  span.style.fontSize = `${fragment.sizeValue}${unit}`;
  span.style.fontWeight = String(fragment.weight);
  span.style.color =
    `rgba(${fragment.r}, ${fragment.g}, ${fragment.b}, ${fragment.a / 255})`;

  // 見えている分と、まだ出ていない分。後者は場所だけ取って見えない。
  const shown = document.createTextNode("");
  const hiddenSpan = document.createElement("span");
  const hidden = document.createTextNode(fragment.text);

  hiddenSpan.style.visibility = "hidden";
  hiddenSpan.appendChild(hidden);
  span.appendChild(shown);
  span.appendChild(hiddenSpan);
  parent.appendChild(span);

  return {
    text: fragment.text,
    speed: fragment.speed,
    revealed: 0,
    shown,
    hidden,
  };
}

/** 先頭から `count` 文字までを見せる。 */
function reveal(fragment: RevealedFragment, count: number): void {
  if (count === fragment.revealed) return;

  fragment.revealed = count;
  fragment.shown.data = fragment.text.slice(0, count);
  fragment.hidden.data = fragment.text.slice(count);
}
