/**
 * エンジンへのシステムコールの定義。
 *
 * ここが std (`library/std/src/game/base_engine.biwa`) との唯一の合意点である。
 * std 側は `@biwa/engine/vm/syscall` としてこれを取り込み、
 * syscall の記述子を組み立てる (= レジスタに積む)。
 * 実際に制御を渡す `yield` は、コンパイラが scene の中に置く。
 */
export const Sys = {
  /** クリックが来るまで scene を止める。中断する。 */
  Wait: 2,
} as const;

// NOTE: ここに載るのは**中断する syscall だけ**である。
// Content API (`sys_content_push_text` / `sys_content_flush` /
// `sys_content_clear`) は中断しないので、std の native が
// `@biwa/engine/api/message` を直接呼ぶ。番号は要らない。
//
// 1 番は消えた `Sys.Write` が使っていた。欠番のままにしてある。

export type SysNumber = (typeof Sys)[keyof typeof Sys];

/** scene が yield する値。VM exit 時にレジスタに積まれているもの。 */
export interface BiwaSyscall {
  sys: number;
  args: unknown[];
}
