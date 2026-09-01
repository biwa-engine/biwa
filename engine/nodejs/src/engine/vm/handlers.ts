import { writeMessage, waitForClick } from "../api/message";
import { Sys } from "./syscall";
import type { SyscallTable } from "./kernel";

/**
 * syscall 番号から実装への対応表。
 *
 * 番号の定義は `syscall.ts` にあり、std と共有している。
 * ここに載っていない syscall を scene が発行した場合は kernel が落とす。
 *
 * ここに来るのは**中断する syscall だけ**である。
 * 中断しない syscall (canvas オブジェクトの操作など) は、
 * std の native が `@biwa/engine/api/*` を直接呼ぶので kernel を通らない。
 *
 * NOTE: `await_transitions` / `sleep` はこの表に無い。
 * TypeScript ターゲットで `yield` を置けるのは今のところ
 * novel statement (`write` / `wait`) の展開先だけで、
 * 任意の関数呼び出しを中断させる手段がコンパイラに無いためである。
 * これらは当面 wasm ターゲット専用になる (`docs/media-object-model.md`)。
 */
export function createSyscallTable(): SyscallTable {
  return {
    [Sys.Write]: (text: string) => writeMessage(text),
    [Sys.Wait]: () => waitForClick(),
  };
}
