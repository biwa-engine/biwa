// probe.js から SharedArrayBuffer を受け取り、Atomics.wait で止まってから起こされるまでを測る。
// エンジンの Worker (src/engine/vm/wasm/worker.ts) がブロッキング syscall でやっていることと同じ。
self.addEventListener("message", (event) => {
  const { buffer } = event.data;
  try {
    const cell = new Int32Array(buffer);
    self.postMessage({ kind: "waiting" });
    const started = performance.now();
    const result = Atomics.wait(cell, 0, 0, 5000);
    self.postMessage({ kind: "woken", result, ms: performance.now() - started, value: Atomics.load(cell, 0) });
  } catch (e) {
    self.postMessage({ kind: "error", message: String(e) });
  }
});
