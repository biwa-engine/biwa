import { engine } from "./context";

/**
 * メッセージウィンドウにテキストを書き足す。
 *
 * `std::game::base_engine::write` (lang item `write`) の実体で、
 * scene 本文に直接書かれたノベルテキストがここに来る。
 * 即座にリターンする非ブロッキング syscall。
 */
export function showMessage(text: string): void {
  const { components, messageBoxId } = engine();
  const box = components.getTextBox(messageBoxId);
  box.show();
  box.appendText(text);
}

/**
 * クリック待ち。`std::game::base_engine::wait` (lang item `wait`) の実体。
 *
 * 本来は「完了までブロックする syscall」だが、現在のコード生成は
 * scene 本文を同期関数として吐くため、JavaScript 側で待つ手段がない。
 * したがって現状は即座にリターンする (= シーンが一気に流れる)。
 * 解決の方向性は engine の README を参照。
 */
export function waitForClick(): void {
  console.warn(
    "[biwa] waitForClick() is not blocking yet: the scene runs to the end without waiting",
  );
}
