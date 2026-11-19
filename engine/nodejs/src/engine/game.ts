/**
 * 走らせるゲーム。`src/game/entry.ts` がページの横の `biwa-game.json` から作る。
 *
 * 生成物は wasm だけである (TypeScript のターゲットは issue #23 で廃止した)。
 * Worker で走らせ、syscall はスレッドを跨ぐ (`vm/wasm/`)。
 */
export interface BiwaGame {
  packageName: string;
  /** `.wasm` の URL。 */
  url: string;
  /** 生成物の内容から決まる値。ブラウザのキャッシュを避けるために付ける。 */
  buildId: string;
}
