import { engine } from "./context";

/**
 * メッセージウィンドウにテキストを書き足す。
 *
 * syscall `Sys.Write` の実装。scene 本文に直接書かれたノベルテキストがここに来る。
 * 中断しないので、そのまま値を返す。
 */
export function writeMessage(text: string): void {
  const { components, messageBoxId } = engine();
  const box = components.getTextBox(messageBoxId);
  box.show();
  box.appendText(text);
}

/**
 * クリックが来るまで待つ。
 *
 * syscall `Sys.Wait` の実装。Promise を返すので kernel はこれを await し、
 * 解決するまで scene を再開しない (= VM は止まったまま)。
 *
 * **進行中の sync 印の演出があれば、クリックはまずそれを完了させる。**
 * テキストを進めるにはもう一度クリックが要る。
 * 独立に走らせている演出 (背景のパン、常時のゆらぎ) はこれに巻き込まれない。
 */
export function waitForClick(): Promise<void> {
  const { host, components, messageBoxId, objects } = engine();
  const box = components.getTextBox(messageBoxId);

  return new Promise((resolve) => {
    const onClick = (): void => {
      if (objects.skipSync()) return;
      host.removeEventListener("click", onClick);
      box.clear();
      resolve();
    };
    host.addEventListener("click", onClick);
  });
}
