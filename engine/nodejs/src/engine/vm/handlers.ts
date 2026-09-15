import { waitForClick } from "../api/message";
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
 * コンパイラが `yield` を置くのは novel statement の展開先だけで、
 * 任意の関数呼び出しを中断させる手段が無いためである。
 *
 * **その `Sys.Wait` すら、いまは TypeScript 経路では届かない。**
 * `sys_wait` を呼ぶのは std の `content_flush_and_wait()` という普通の関数で、
 * statement の位置には無いので `yield` が置かれない
 * (`docs/content-api.md` の段 2)。TypeScript は tier 2 なので当面このままである。
 */
export function createSyscallTable(): SyscallTable {
  return {
    [Sys.Wait]: () => waitForClick(),
  };
}
