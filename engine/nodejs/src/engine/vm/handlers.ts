import { writeMessage, waitForClick } from "../api/message";
import { Sys } from "./syscall";
import type { SyscallTable } from "./kernel";

/**
 * syscall 番号から実装への対応表。
 *
 * 番号の定義は `syscall.ts` にあり、std と共有している。
 * ここに載っていない syscall を scene が発行した場合は kernel が落とす。
 */
export function createSyscallTable(): SyscallTable {
  return {
    [Sys.Write]: (text: string) => writeMessage(text),
    [Sys.Wait]: () => waitForClick(),
  };
}
