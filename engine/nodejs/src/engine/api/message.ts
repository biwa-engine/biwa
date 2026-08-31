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
 */
export function waitForClick(): Promise<void> {
  const { host, components, messageBoxId } = engine();
  const box = components.getTextBox(messageBoxId);

  return new Promise((resolve) => {
    host.addEventListener(
      "click",
      () => {
        box.clear();
        resolve();
      },
      { once: true },
    );
  });
}
