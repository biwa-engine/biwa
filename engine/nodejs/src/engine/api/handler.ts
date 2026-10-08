/**
 * TypeScript ターゲットのハンドラの表 (`docs/host-function-values.md`)。
 *
 * TypeScript ターゲットではゲームコードもメインスレッドで走るので、
 * Biwa から渡された関数はここで預かる。wasm ターゲットでは Worker が持つ
 * (`vm/wasm/worker.ts`) ので、ここは使わない。
 */

import { type Handler, HandlerTable } from "../vm/handlerTable";

const table = new HandlerTable();

/**
 * 関数を預かり、その番号を返す。
 *
 * std の `sys_ui_set_handler` (TypeScript 版) が、syscall に渡す前に関数を番号に替えるのに使う。
 */
export function retainHandler(f: Handler): number {
  return table.retain(f);
}

/** 番号から関数を引く。 */
export function lookupHandler(handle: number): Handler | undefined {
  return table.get(handle);
}

/** 関数を手放す。持ち主の Element が消えたときに `UIObjects` から呼ばれる。 */
export function releaseHandlers(handles: number[]): void {
  table.release(handles);
}
