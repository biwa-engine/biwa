import type { ComponentRegistry } from "../../components/ComponentRegistry";
import type { CanvasObjects } from "../canvas/CanvasObjects";
import type { Renderer } from "../Renderer";
import type { UIObjects } from "../ui/UIObjects";

/**
 * syscall の実装が触るエンジンの実体。
 *
 * 生成されたゲームコードは std 経由でしかエンジンに触れず、
 * std の native 実装も syscall の実装もエンジンを引数で受け取らない。
 * そのため、起動時に一度だけ登録したこのコンテキストを参照する。
 */
export interface EngineContext {
  renderer: Renderer;
  components: ComponentRegistry;
  /** canvas に置かれたオブジェクトと、その遷移。 */
  objects: CanvasObjects;
  /** UI Element のツリー (`docs/ui-api.md`)。 */
  ui: UIObjects;
  /** 入力を受け取る要素 (クリック待ちの対象) */
  host: HTMLElement;
  /** メッセージウィンドウとして使うコンポーネントの ID */
  messageBoxId: string;
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
