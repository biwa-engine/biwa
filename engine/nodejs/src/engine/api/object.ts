import { engine } from "./context";

/**
 * canvas オブジェクトの syscall の実装。
 *
 * どのターゲットから来てもここに合流する
 * (`vm/handlers.ts` と `vm/wasm/host.ts` の両方がこれを呼ぶ)。
 * パスはパッケージの `assets/` を基準とした相対パスである (`api/assets.ts`)。
 *
 * 中断しない syscall は失敗してもゲームを止めず、ログを出すに留める。
 * 積んで直ちに返る API なので、投げても呼び出し元には届かないからである。
 *
 * 規約と設計は `docs/media-object-model.md` にある。
 */

/**
 * 新しいオブジェクト id を採る。
 *
 * TypeScript ターゲット用。wasm ターゲットでは Worker が自分で採番する
 * (メインスレッドと往復せずに id を返すため)。
 */
export function allocObjectId(): number {
  return engine().objects.allocId();
}

/**
 * オブジェクトを作る。
 *
 * `w` / `h` が負なら「指定しない」で、テクスチャの元のサイズから決まる。
 * 片方だけ正ならアスペクトを保つ。
 */
export function createObject(
  id: number,
  path: string,
  layer: number,
  x: number,
  y: number,
  w: number,
  h: number,
  alpha: number,
  theta: number,
): void {
  engine().objects.create(id, path, layer, x, y, w, h, alpha, theta);
}

/** `after` ミリ秒後にオブジェクトを消す。 */
export function deleteObject(id: number, after: number): void {
  engine().objects.remove(id, after);
}

/** 次の `startTransitions` に備えて遷移を積む。積むだけで何も起きない。 */
export function addTransition(
  id: number,
  param: number,
  kind: number,
  value: number,
  after: number,
  duration: number,
): void {
  engine().objects.addTransition(id, param, kind, value, after, duration);
}

/**
 * 積まれた遷移を発火する。オブジェクトを跨いで一斉に始まる。
 *
 * `sync` が 0 でなければ、この演出はテキストの進行と結びつく。
 * クリックはまずこれを完了させ、`awaitTransitions` が待つ対象にもなる。
 */
export function startTransitions(sync: number): void {
  engine().objects.start(sync !== 0);
}

/** sync 印の演出がすべて終わるまでシーンを止める。 */
export function awaitTransitions(): Promise<void> {
  return engine().objects.awaitSync();
}

/** エンジン時計の上で `ms` ミリ秒シーンを止める (ポーズ中は進まない)。 */
export function sleep(ms: number): Promise<void> {
  return engine().objects.sleep(ms);
}
