import type { ComponentRegistry } from "../../components/ComponentRegistry";
import type { Renderer } from "../Renderer";

/**
 * syscall 層 (`@biwa/engine/*`) が触るエンジンの実体。
 *
 * 生成されたゲームコードは `std` 経由でしかエンジンに触れず、
 * `std` の native 実装は引数としてエンジンを受け取らない。
 * そのため、起動時に一度だけ登録したこのコンテキストを参照する。
 */
export interface EngineContext {
  renderer: Renderer;
  components: ComponentRegistry;
  /** メッセージウィンドウとして使うコンポーネントの ID */
  messageBoxId: string;
  /** キャラクター・立ち絵を載せる canvas レイヤーの ID */
  spriteLayerId: string;
}

let current: EngineContext | null = null;

export function setEngineContext(ctx: EngineContext): void {
  current = ctx;
}

export function engine(): EngineContext {
  if (current === null) {
    throw new Error(
      "[biwa] engine is not initialized: setEngineContext() must be called before the game runs",
    );
  }
  return current;
}
