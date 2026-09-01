/**
 * wasm 生成物とエンジンの境界の定義。
 *
 * ここが std (`library/std/src/game/base_engine.biwa` の `[[native(arch="wasm")]]`)
 * との唯一の合意点である。TypeScript 経路における `vm/syscall.ts` に相当する。
 *
 * std は `(import "biwa:engine" "sys_*" ...)` を自分で書き、コンパイラはそれを
 * そのまま生成物の先頭に置く。つまり import の名前空間と名前は
 * std とエンジンの取り決めであって、コンパイラは関知しない。
 */

/** std が書くエンジン API の import 名前空間。 */
export const ENGINE_NAMESPACE = "biwa:engine";

/**
 * コンパイラ自身が要求する import の名前空間。
 *
 * std ではなく biwac が出す。今のところ文字列リテラルの実体化だけである
 * (データセグメント上のバイト列を JS 文字列にする)。
 */
export const RUNTIME_NAMESPACE = "biwa:runtime";

/**
 * syscall の実行のされ方。
 *
 * wasm 本体は Worker で走り、描画はメインスレッドにある。
 * どこで実行するか、そして呼び出し元を止めるかどうかがこの区分である。
 * wasm 側から見ればどれも普通の同期呼び出しで、違いは見えない。
 */
export type SyscallKind =
  /**
   * Worker 内で同期実行する。
   *
   * エンジンには用が無く、wasm の値 (externref / anyref) を触るだけのもの。
   * これらの値は JS のオブジェクト参照そのものなのでスレッド境界を越えられない。
   * 越えられないから Worker に留めるのであって、速いからではない。
   */
  | "local"
  /**
   * メインスレッドへ投げて、結果を待たずに返る。
   *
   * 「処理をキューに積んで直ちにリターンする」API。
   * 同じポートへの postMessage は順序が保たれるので、
   * 後続の syscall との前後関係は崩れない。
   */
  | "cast"
  /**
   * メインスレッドへ投げるが、戻り値 (新しい id) は Worker 内で採番して返す。
   *
   * `cast` と同じく止まらないのに戻り値を持てる。
   * 素直に `call` にすると、オブジェクトを 1 つ作るたびに
   * `Atomics.wait` でスレッドが往復してしまう。
   * 採番は単調増加なので決定的で、セーブ・ロードの記録再生とも噛み合う。
   */
  | "alloc"
  /**
   * メインスレッドへ投げて、完了するまで Worker を止める。
   *
   * クリック待ちのように完了までブロックする API。
   * 止まるのは Worker だけなので、描画は動き続ける。
   */
  | "call";

/**
 * `biwa:engine` の各 import をどう捌くか。
 *
 * ここに載っていない import を生成物が要求したら、Worker は
 * その名前を挙げて失敗する (不透明な LinkError にはしない)。
 */
export const ENGINE_SYSCALLS: Record<string, SyscallKind> = {
  /** メッセージウィンドウにテキストを書く。積んで返る。 */
  sys_write: "cast",
  /** クリックが来るまで止まる。 */
  sys_wait: "call",

  /** canvas にオブジェクトを置く。id を返すが、止まらない。 */
  sys_create_object: "alloc",
  /** オブジェクトを消す。 */
  sys_delete_object: "cast",
  /** 次の発火に備えて遷移を積む。 */
  sys_add_transition: "cast",
  /** 積まれた遷移を発火する。 */
  sys_start_transitions: "cast",
  /** sync 印の演出が終わるまで止まる。 */
  sys_await_transitions: "call",
  /** エンジン時計の上で指定時間止まる。 */
  sys_sleep: "call",

  // 以下はエンジン呼び出しではなく、wasm の値に対する操作である。
  // 生成物は `String` を externref、`Map` / `Option` を externref / anyref として
  // 持っているので、その実体は JS 側にしか無い。
  /** 文字列の連結。 */
  sys_string_concat: "local",
  /** Map への挿入。 */
  sys_map_insert: "local",
  /** Map からの取得。 */
  sys_map_get: "local",
};

/**
 * cast したあと直ちに送り出す syscall。
 *
 * cast はまとめて 1 通の postMessage で流している (`bridge.ts`)。
 * 遷移の発火だけは待たせたくないので、ここで区切る。
 * ついでにバッチが 1 通に収まるので、**メインスレッドが発火の途中で
 * フレームを描けない** — 揃って始まることが構造として保証される。
 */
export const FLUSH_AFTER_CAST: ReadonlySet<string> = new Set([
  "sys_start_transitions",
]);
