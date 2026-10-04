import { engine } from "./context";

/**
 * Message Window へ content を積む。
 *
 * syscall `sys_content_push_text` の実装。積むだけで描画は始まらない。
 * 引数はすべて std が設定を解決した**絶対値**である
 * (`docs/content-api.md` の決めたこと 3)。エンジンは設定を持たない。
 *
 * Content API の syscall はすべて第一引数 `uiId` で出力先の
 * UI Element `MessageArea` を指定する (`docs/ui-api-impl-status.md` §14)。
 * MessageArea でない ui_id が来たら叱って捨てる (中断しない syscall なので投げても届かない)。
 */
export function pushContentText(
  uiId: number,
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
  engine()
    .ui.messageArea(uiId)
    ?.push({ text, speed, sizeUnit, sizeValue, weight, r, g, b, a });
}

/**
 * 積まれた content を出し始める。
 *
 * syscall `sys_content_flush` の実装。
 */
export function flushContent(uiId: number): void {
  engine().ui.messageArea(uiId)?.flush();
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
export function clearContent(uiId: number): void {
  engine().ui.messageArea(uiId)?.clear();
}

/**
 * クリックが来るまで待つ。
 *
 * syscall `sys_wait` の実装。Promise を返すので kernel はこれを await し、
 * 解決するまで scene を再開しない (= VM は止まったまま)。
 *
 * **進行中のものがあれば、クリックはまずそれを完了させる。**
 * 文字送りの途中なら残りを全部出し、sync 印の演出が走っていれば終端へ飛ばす。
 * テキストを進めるにはもう一度クリックが要る。
 * 独立に走らせている演出 (背景のパン、常時のゆらぎ) はこれに巻き込まれない。
 *
 * 2 つを 1 回のクリックで畳むのは、両方が走っているときに
 * 3 回クリックさせないためである。利用者から見た規則は
 * 「進行中のものがあれば 1 回目で畳み、次で進む」で一貫する。
 */
export function waitForClick(): Promise<void> {
  const { host, ui, objects } = engine();

  return new Promise((resolve) => {
    const onClick = (): void => {
      // 短絡させない。どちらも必ず試す。
      // クリックはどの出力先にも属さないので、文字送りはすべての MessageArea で畳む。
      const skippedText = ui.skipMessageAreas();
      const skippedSync = objects.skipSync();
      if (skippedText || skippedSync) return;

      host.removeEventListener("click", onClick);
      resolve();
    };
    host.addEventListener("click", onClick);
  });
}
