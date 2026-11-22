// 同じ game.wasm を V8 (node) で動かし、ネイティブのプレイヤー (wasmtime) と比べるためのもの。
//
//   node bench/run-v8.mjs <game.wasm>
//
// エンジン (描画・クリック待ち) は持たない。止まる syscall はすぐ返し、
// 止まらない syscall は捨てる。wasm の値に対する操作 (文字列・Vec・Map) だけを
// `engine/nodejs/src/engine/vm/wasm/worker.ts` と同じ形で実装する。
// 測るのは `biwa-runtime --headless` の統計と同じもの (wasm の実行時間・ホスト呼び出しの間隔)。

import { readFileSync } from "node:fs";

const path = process.argv[2];
if (!path) {
  console.error("usage: node run-v8.mjs <game.wasm>");
  process.exit(2);
}

const stats = { hostCalls: 0, stringsCreated: 0, running: 0, maxGap: 0, gapsOver1ms: 0 };
let lastReturn = performance.now();
const traced = (f) => (...args) => {
  const gap = performance.now() - lastReturn;
  stats.hostCalls += 1;
  stats.running += gap;
  if (gap > stats.maxGap) stats.maxGap = gap;
  if (gap > 1) stats.gapsOver1ms += 1;
  const r = f(...args);
  lastReturn = performance.now();
  return r;
};

let memory = null;
let nextId = 1;
let nextHandle = 1;
const handlers = new Map();
const nextSound = { bus: 1, effect: 1, object: 1 };
const ui = { onClick: null, mainScene: null, windowHandle: null };
const decoder = new TextDecoder();

const engine = {
  sys_content_push_text: () => {},
  sys_content_flush: () => {},
  sys_content_clear: () => {},
  sys_wait: () => {},
  sys_create_object: () => nextId++,
  sys_delete_object: () => {},
  sys_add_transition: () => {},
  sys_start_transitions: () => {},
  sys_await_transitions: () => {},
  sys_sleep: () => {},
  sys_string_concat: (a, b) => { stats.stringsCreated += 1; return a + b; },
  sys_string_eq: (a, b) => (a === b ? 1 : 0),
  sys_int_to_string: (v) => { stats.stringsCreated += 1; return String(v); },
  sys_float_to_string: (v) => { stats.stringsCreated += 1; return String(Number(v.toPrecision(9))); },
  sys_vec_new: () => [],
  sys_vec_of: (v) => [v],
  sys_vec_push: (vec, v) => { vec.push(v); },
  sys_vec_len: (vec) => vec.length,
  sys_vec_get: (vec, i) => (i < vec.length ? vec[i] : null),
  sys_map_insert: (map, k, v) => { map.set(k, v); },
  sys_map_get: (map, k) => map.get(k) ?? null,
  sys_ui_create: () => nextId++,
  sys_ui_set_property: () => {},
  sys_ui_set_property_with_string: () => {},
  sys_ui_push_child: () => {},
  sys_ui_set_handler: (id, kind, f) => {
    const handle = nextHandle++;
    handlers.set(handle, f);
    if (kind === 0) ui.mainScene ??= f;
    if (kind === 1) ui.onClick ??= f;
  },
  sys_ui_set_value: (id, kind, v) => {
    if (kind === 0) ui.windowHandle ??= v;
  },
  // 音は鳴らさない。id を空間ごとに振るだけ。
  sys_sound_bus_create: () => nextSound.bus++,
  sys_sound_effect_create: () => nextSound.effect++,
  sys_sound_bus_add_effect: () => {},
  sys_sound_bus_attach: () => {},
  sys_sound_output_attach: () => {},
  sys_sound_bus_add_stop_all: () => {},
  sys_sound_bus_delete: () => {},
  sys_sound_bus_add_transition: () => {},
  sys_sound_bus_set_param: () => {},
  sys_sound_effect_add_transition: () => {},
  sys_sound_effect_set_param: () => {},
  sys_sound_create: () => nextSound.object++,
  sys_sound_add_start: () => {},
  sys_sound_add_stop: () => {},
  sys_sound_add_transition: () => {},
  sys_sound_preload: () => {},
};

const tracedEngine = Object.fromEntries(Object.entries(engine).map(([k, f]) => [k, traced(f)]));
const runtime = {
  string_const: traced((offset, length) => {
    stats.stringsCreated += 1;
    return decoder.decode(new Uint8Array(memory.buffer, offset, length));
  }),
};

const compileStart = performance.now();
const module = await WebAssembly.compile(readFileSync(path));
const compile = performance.now() - compileStart;
const { exports } = await WebAssembly.instantiate(module, {
  "biwa:engine": tracedEngine,
  "biwa:runtime": runtime,
});
memory = exports.memory;

const run = (f, ...args) => {
  lastReturn = performance.now();
  const r = f(...args);
  stats.running += performance.now() - lastReturn;
  return r;
};

const started = performance.now();
run(exports.__biwa_entrypoint);
const game = run(ui.onClick, ui.windowHandle);
run(ui.mainScene, game);
const total = performance.now() - started;

console.log("[v8] the scene finished");
console.log(`  compile          ${compile.toFixed(1)} ms`);
console.log(`  wasm running     ${stats.running.toFixed(1)} ms`);
console.log(`  host calls       ${stats.hostCalls}`);
console.log(`  strings created  ${stats.stringsCreated}`);
console.log(`  max gap          ${stats.maxGap.toFixed(3)} ms`);
console.log(`  gaps over 1 ms   ${stats.gapsOver1ms}`);
console.log(`  total            ${total.toFixed(1)} ms`);
