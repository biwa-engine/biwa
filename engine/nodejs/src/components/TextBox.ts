export class TextBox {
  readonly element: HTMLElement;
  private textEl: HTMLParagraphElement;

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

  setText(text: string): void {
    this.textEl.textContent = text;
  }

  appendText(text: string): void {
    this.textEl.textContent += text;
  }

  clear(): void {
    this.textEl.textContent = "";
  }
}
