import { engine } from "./context";

/**
 * Message Window へ content を積む。
 *
 * syscall `sys_content_push_text` の実装。積むだけで描画は始まらない。
 * 引数はすべて std が設定を解決した**絶対値**である
 * (`docs/content-api.md` の決めたこと 3)。エンジンは設定を持たない。
 */
export function pushContentText(
  text: string,
  speed: number,
  sizeUnit: number,
  sizeValue: number,
  weight: number,
  r: number,
  g: number,
  b: number,
  a: number,
): void {
  const { components, messageBoxId } = engine();
  const box = components.getTextBox(messageBoxId);
  box.show();
  box.push({ text, speed, sizeUnit, sizeValue, weight, r, g, b, a });
}

/**
 * 積まれた content を出し始める。
 *
 * syscall `sys_content_flush` の実装。
 */
export function flushContent(): void {
  const { components, messageBoxId } = engine();
  components.getTextBox(messageBoxId).flush();
}

/**
 * 枠を空にする。
 *
 * syscall `sys_content_clear` の実装。
 *
 * **エンジンは自分の判断でクリアしない。** 呼ぶ時機を決めるのは std で、
 * いまは `content_flush_and_wait()` がクリック待ちから戻った直後に呼ぶ。
 * 「待つが消さない」API を将来足すときに、
 * 変更がコンパイラと std に閉じるようにするための切り分けである。
 */
export function clearContent(): void {
  const { components, messageBoxId } = engine();
  components.getTextBox(messageBoxId).clear();
}

/**
 * クリックが来るまで待つ。
 *
 * syscall `sys_wait` の実装。Promise を返すので kernel はこれを await し、
 * 解決するまで scene を再開しない (= VM は止まったまま)。
 *
 * **進行中の sync 印の演出があれば、クリックはまずそれを完了させる。**
 * テキストを進めるにはもう一度クリックが要る。
 * 独立に走らせている演出 (背景のパン、常時のゆらぎ) はこれに巻き込まれない。
 *
 * TODO(段 4): 文字送り中のクリックは、まず送りを最後まで飛ばす。
 */
export function waitForClick(): Promise<void> {
  const { host, objects } = engine();

  return new Promise((resolve) => {
    const onClick = (): void => {
      if (objects.skipSync()) return;
      host.removeEventListener("click", onClick);
      resolve();
    };
    host.addEventListener("click", onClick);
  });
}
