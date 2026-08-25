// Biwa 実行モデル PoC: generator を VM、yield を syscall 命令として使う。
//
// - `scene` は generator function にコンパイルされる
// - syscall は `yield { sys, args }` (= レジスタに積んで VM exit)
// - ホスト (kernel) が処理し、`gen.next(result)` で結果を書き戻して再開
// - ブロッキング syscall は handler が Promise を返すだけ

const SYS = { WRITE: 1, WAIT: 2, CHOICE: 3 };

// ---------------------------------------------------------------- kernel side

class Vm {
  constructor(handlers) {
    this.handlers = handlers;
    this.trace = []; // syscall の結果ログ (セーブに使う)
  }

  /**
   * VM を回す。
   * replay を渡すと、その分だけ副作用を起こさずログの結果を食わせて早送りする。
   */
  async run(gen, replay = []) {
    let send;
    let i = 0;
    for (;;) {
      const { value, done } = gen.next(send);
      if (done) return { returned: value, trace: this.trace };

      const { sys, args } = value; // ← "レジスタ" の中身

      if (i < replay.length) {
        // 復元中: 実際の処理はせず、記録済みの戻り値をそのまま書き戻す
        send = replay[i];
        this.trace.push(replay[i]);
        i += 1;
        continue;
      }

      const result = await this.handlers[sys](...args);
      this.trace.push(result ?? null);
      send = result;
      i += 1;
    }
  }
}

// ------------------------------------------------------- "generated" std side
// biwac が std の syscall 宣言から吐く想定のラッパー

function* write(msg) {
  return yield { sys: SYS.WRITE, args: [msg] };
}
function* wait() {
  return yield { sys: SYS.WAIT, args: [] };
}
function* choice(options) {
  return yield { sys: SYS.CHOICE, args: [options] };
}

// ------------------------------------------------ "generated" game code side
// syscall しない関数は generator にしない (色付けの結果)

function add(x, y) {
  return x + y;
}

function* scene_main(g) {
  yield* write("Hello, World!");
  yield* wait();
  yield* write("こんにちは、Biwaの世界。");
  yield* wait();

  const x = add(1, 2);
  if (x > 2) {
    yield* write("Nice!");
    yield* wait();
  }

  // プレイヤー入力に依存する分岐。command queue 方式では原理的に書けない。
  const drink = yield* choice(["紅茶", "珈琲"]);
  g.states.drink = drink;
  yield* write(`${drink} を選んだ。`);
  yield* wait();

  return g;
}

// ----------------------------------------------------------------- host (engine)

function makeEngine({ clicks, picks }) {
  const screen = [];
  let clickIndex = 0;
  let pickIndex = 0;

  const handlers = {
    [SYS.WRITE]: (msg) => {
      screen.push(msg);
      console.log(`  [screen] ${msg}`);
    },
    // ブロッキング syscall: クリックが来るまで再開しない
    [SYS.WAIT]: () =>
      new Promise((resolve) => {
        const delay = clicks[clickIndex++] ?? 0;
        console.log(`  [kernel] waiting for click (${delay}ms)`);
        setTimeout(resolve, delay);
      }),
    [SYS.CHOICE]: (options) =>
      new Promise((resolve) => {
        const picked = options[picks[pickIndex++] ?? 0];
        console.log(`  [kernel] choice ${JSON.stringify(options)} -> ${picked}`);
        setTimeout(() => resolve(picked), 5);
      }),
  };

  return { handlers, screen };
}

// ------------------------------------------------------------------ run demo

console.log("=== 1. 通常実行 ===");
const engine1 = makeEngine({ clicks: [5, 5, 5, 5], picks: [1] });
const vm1 = new Vm(engine1.handlers);
const game1 = { name: "test1", characters: {}, states: {}, window: {} };
const result1 = await vm1.run(scene_main(game1));
console.log("returned states:", result1.returned.states);
console.log("trace:", JSON.stringify(result1.trace));

// セーブ: シーンの途中まで走らせて、そこまでの syscall ログと Game を保存する。
console.log("\n=== 2. 途中でセーブ (3 回目の wait まで) ===");
const engine2 = makeEngine({ clicks: [5, 5, 5, 5], picks: [1] });
const vm2 = new Vm(engine2.handlers);
const game2 = { name: "test1", characters: {}, states: {}, window: {} };

// 途中で止めるために、n 回 syscall したらキャンセルする版を回す
const partialTrace = [];
{
  const gen = scene_main(game2);
  let send;
  for (let i = 0; i < 6; i += 1) {
    const { value, done } = gen.next(send);
    if (done) break;
    const r = await engine2.handlers[value.sys](...value.args);
    partialTrace.push(r ?? null);
    send = r;
  }
  console.log("saved:", JSON.stringify({ scene: "main", trace: partialTrace }));
}

console.log("\n=== 3. ロード (再実行 + ログ再生で同じ地点へ) ===");
const engine3 = makeEngine({ clicks: [5, 5], picks: [0] });
const vm3 = new Vm(engine3.handlers);
const game3 = { name: "test1", characters: {}, states: {}, window: {} };
const result3 = await vm3.run(scene_main(game3), partialTrace);
console.log("returned states:", result3.returned.states);
console.log(
  "画面に出た行 (復元後に実際に描画されたものだけ):",
  JSON.stringify(engine3.screen),
);

// ------------------------------------------------------------------ overhead

console.log("\n=== 4. yield* の実測コスト ===");

function* leaf(n) {
  return yield { sys: SYS.WRITE, args: [n] };
}
function* body(n) {
  let acc = 0;
  for (let i = 0; i < n; i += 1) acc += yield* leaf(i);
  return acc;
}
function plainLeaf(n) {
  return n;
}
function plainBody(n) {
  let acc = 0;
  for (let i = 0; i < n; i += 1) acc += plainLeaf(i);
  return acc;
}

const N = 1_000_000;

{
  const t0 = performance.now();
  const gen = body(N);
  let send;
  for (;;) {
    const { value, done } = gen.next(send);
    if (done) break;
    send = value.args[0]; // 同期 handler 相当
  }
  const t1 = performance.now();
  console.log(`generator + syscall x${N}: ${(t1 - t0).toFixed(1)}ms`);
}
{
  const t0 = performance.now();
  plainBody(N);
  const t1 = performance.now();
  console.log(`plain function call x${N}: ${(t1 - t0).toFixed(1)}ms`);
}
