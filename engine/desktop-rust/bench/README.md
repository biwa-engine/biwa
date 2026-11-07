# ベンチマーク

## gc-churn: wasm ランタイムの GC

scene の中で文字列と struct を大量に作っては捨て、struct を Vec に積み続けるゲーム
(ホスト呼び出し 約 600 万回、文字列 約 590 万個)。

```sh
cd bench/gc-churn && biwa build --platform web      # .biwa_build/wasm/gcchurn.wasm ができる
cd ../.. && cargo build --release -p biwa-runtime

# wasmtime (ネイティブのプレイヤー)。--collector で GC の方式を変えられる (auto / drc / null)
./target/release/biwa-runtime --headless --wasm bench/gc-churn/.biwa_build/wasm/gcchurn.wasm

# 同じ wasm を V8 (node) で。比較用
node bench/run-v8.mjs bench/gc-churn/.biwa_build/wasm/gcchurn.wasm
```

どちらも同じ統計を出す: wasm の実行時間 (ホスト呼び出しの間の時間の合計)、
ホスト呼び出しの間隔の最大 (GC の停止はここに出る)、全体の時間。
結果は `../README.md` の「段 2」にある。
