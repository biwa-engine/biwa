/**
 * wasm 生成物とエンジンの境界の定義。
 *
 * ここが std (`library/std/src/game/runtime.biwa` の `[[native(arch="wasm")]]`)
 * との唯一の合意点である。
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
   * `cast` と同じく積んで返るが、決まった位置の引数 (`RETAINED_ARG`) を Worker の表に預け、
   * その番号 (handle) に替えてから投げる。
   *
   * 預けるのは関数 (ハンドラ) か値 (WasmGC の struct)。どちらも `postMessage` できないので、
   * メインスレッドには番号しか届かない。本体は Worker に残り、ホストが後で Worker の中から
   * 呼ぶ (関数) か、host export の引数として返す (値) (`docs/host-function-values.md`)。
   */
  | "retain"
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
  // Content API の 3 つは第一引数が出力先の MessageArea の ui_id。
  /** Message Window に content を積む。描画は始まらない。 */
  sys_content_push_text: "cast",
  /** 積まれた content を出し始める。 */
  sys_content_flush: "cast",
  /** 枠を空にする。時機を決めるのは std であってエンジンではない。 */
  sys_content_clear: "cast",
  /** クリックが来るまで止まる。 */
  sys_timeline_wait_click: "call",

  /** canvas にオブジェクトを置く。id を返すが、止まらない。 */
  sys_canvas_object_create: "alloc",
  /** オブジェクトを消す。 */
  sys_canvas_object_delete: "cast",
  /** 次の発火に備えて遷移を積む。 */
  sys_canvas_object_add_transition: "cast",
  /** 積まれた遷移を発火する。 */
  sys_timeline_fire: "cast",
  /** sync 印の演出が終わるまで止まる。 */
  sys_timeline_await_synced: "call",
  /** エンジン時計の上で指定時間止まる。 */
  sys_timeline_sleep: "call",

  // 以下はエンジン呼び出しではなく、wasm の値に対する操作である。
  // 生成物は `String` を externref、`Map` / `Option` を externref / anyref として
  // 持っているので、その実体は JS 側にしか無い。
  /** 文字列の連結。 */
  sys_string_concat: "local",
  /** 文字列の比較 (中身で)。 */
  sys_string_eq: "local",
  /** 数値から文字列を作る。`String` の実体はホスト側にしか無い。 */
  sys_int_to_string: "local",
  sys_float_to_string: "local",
  /** 可変長配列。中身は wasm の値なので Worker 側に置く。 */
  sys_vec_new: "local",
  sys_vec_of: "local",
  sys_vec_push: "local",
  sys_vec_len: "local",
  /** 範囲外なら null (= `Option::none`) を返す。 */
  sys_vec_get: "local",
  /** Map への挿入。 */
  sys_map_insert: "local",
  /** Map からの取得。 */
  sys_map_get: "local",

  // UI Element (`docs/ui-api.md`)。kind の番号は `api/ui.ts` が正で、
  // wasm 側の std はそれを `.wat` に直書きしている。
  /** UI Element を作る。id を返すが、止まらない。 */
  sys_ui_create: "alloc",
  /** 数値の property を設定する。 */
  sys_ui_set_property: "cast",
  /** 文字列を伴う property を設定する。 */
  sys_ui_set_property_with_string: "cast",
  /** 子 Element を親に積む。 */
  sys_ui_push_child: "cast",
  /** Element にハンドラ (ホストが後で呼ぶ関数) を設定する。 */
  sys_ui_set_handler: "retain",
  /** Element に値 (ホストが中身を見ずに預かり、後で返す struct) を設定する。 */
  sys_ui_set_value: "retain",

  // 音 (`docs/sound.md`)。id は SoundObject・Bus・Effect の空間ごとに Worker で採番する (`ALLOC_SPACE`)。
  /** バスを作る。 */
  sys_sound_bus_create: "alloc",
  /** バスの effect の列の末尾に、作ってある effect を挿す (繋ぐ前だけ)。 */
  sys_sound_bus_add_effect: "cast",
  sys_sound_bus_attach: "cast",
  /** 根のバスを出力に繋ぐ。根は 1 本だけ。 */
  sys_sound_output_attach: "cast",
  /** バスの部分木で鳴っている音をすべて止める (発火で)。 */
  sys_sound_bus_add_stop_all: "cast",
  /** `after` の後に、バスとその部分木を消す。 */
  sys_sound_bus_delete: "cast",
  sys_sound_bus_add_transition: "cast",
  /** 即座に値を設定する (発火を待たない)。 */
  sys_sound_bus_set_param: "cast",
  /** effect を作る (まだどのバスにも挿さない)。effect の id を返す。 */
  sys_sound_effect_create: "alloc",
  sys_sound_effect_add_transition: "cast",
  sys_sound_effect_set_param: "cast",
  /** SoundObject を作る。まだ鳴らない (開始は add_start + start_transitions)。 */
  sys_sound_object_create: "alloc",
  sys_sound_object_add_start: "cast",
  sys_sound_object_add_stop: "cast",
  sys_sound_object_add_transition: "cast",
  /** 音のファイルの読み込みを先に始める。 */
  sys_sound_preload: "cast",
};

/**
 * `alloc` の syscall がどの id 空間で採番するか。載っていないものは canvas オブジェクトと UI Element の共通の空間。
 *
 * 音は SoundObject・Bus・Effect を別の空間にする (`docs/sound.md`。それぞれ 1 から振る)。
 */
export const ALLOC_SPACE: Record<string, string> = {
  sys_sound_object_create: "sound",
  sys_sound_bus_create: "bus",
  sys_sound_effect_create: "effect",
};

/**
 * `retain` の syscall で、Worker の表に預ける引数の位置 (0 始まり)。
 *
 * 型で見分けず位置で決めるのは、WasmGC の struct が JS からは `Map` などの externref と
 * 同じ「ただのオブジェクト」に見え、区別できないからである。
 */
export const RETAINED_ARG: Record<string, number> = {
  sys_ui_set_handler: 2,
  sys_ui_set_value: 2,
};

/**
 * Worker 自身がメインスレッドへ流す cast の名前 (std の import ではない)。
 *
 * scene を始めるとき、`on_click` が `Game[S]` を返した後・`main_scene` を呼ぶ前に
 * ScenePage を見せる (引数は Window の ui_id)。`on_click` の中の syscall との前後が崩れないよう、
 * 同じ cast の列に乗せる。
 */
export const ENTER_SCENE_PAGE = "__biwa_enter_scene_page";

/**
 * cast したあと直ちに送り出す syscall。
 *
 * cast はまとめて 1 通の postMessage で流している (`bridge.ts`)。
 * 遷移の発火だけは待たせたくないので、ここで区切る。
 * ついでにバッチが 1 通に収まるので、**メインスレッドが発火の途中で
 * フレームを描けない** — 揃って始まることが構造として保証される。
 */
export const FLUSH_AFTER_CAST: ReadonlySet<string> = new Set([
  "sys_timeline_fire",
]);

// NOTE: `sys_content_flush` はここに要らない。直後に `sys_timeline_wait_click` が来て、
// `call` は必ず溜めてある cast を先に流すためである (`bridge.ts`)。
// 「flush はするが待たない」API を std が持ったら、ここに足すこと。
