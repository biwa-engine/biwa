/**
 * エンジンへのシステムコールの定義。
 *
 * ここが std (`library/std/src/game/base_engine.biwa`) との唯一の合意点である。
 * std 側は `@biwa/engine/vm/syscall` としてこれを取り込み、
 * syscall の記述子を組み立てる (= レジスタに積む)。
 * 実際に制御を渡す `yield` は、コンパイラが scene の中に置く。
 */
export const Sys = {
  /** メッセージウィンドウにテキストを書く。中断しない。 */
  Write: 1,
  /** クリックが来るまで scene を止める。中断する。 */
  Wait: 2,
} as const;

export type SysNumber = (typeof Sys)[keyof typeof Sys];

/** scene が yield する値。VM exit 時にレジスタに積まれているもの。 */
export interface BiwaSyscall {
  sys: number;
  args: unknown[];
}
