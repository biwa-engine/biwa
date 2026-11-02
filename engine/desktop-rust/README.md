# Biwa ネイティブプレイヤー (Rust)

Biwa のゲーム (`game.wasm`) を、OS の WebView に頼らずに動かすデスクトップのプレイヤー。
wasm ランタイム (wasmtime) を同梱するので、WasmGC が動くかどうかが利用者の環境に左右されない。
方針とクレートの選定は `~/memo/biwa-native-runtime-rust.md` (以下「メモ」)、
配布の仕組み (ゲームを焼き込まないプレイヤーを CI でビルドしておく) は `docs/biwa-prebuilt-player.md`。

状況: メモ §11 の **段 1 (wasmtime で動かす) と段 2 (GC の性能) が済み**。段 3 (文字) は作業中。

## crate の構成

外部の crate は、それを担当する crate の中に閉じ込める。
上位の crate (`core`・`player`) から、下位の具体的な crate (wgpu・parley・winit ...) の型は見えない。
例外は wasmtime で、差し替える見込みがほぼ無いので `vm` の公開 API に出てよい
(それでも `player` が直接触らずに済むよう、GC の方式などは `vm` の型で包んでいる)。

```
player (bin) ── つなぐだけ。外部 crate に依存しない
 ├ core       エンジンの論理と、文字・描画のインターフェース。外部 crate に依存しない
 ├ vm         wasmtime
 ├ text       parley + swash            (core::text のトレイトを実装する)
 ├ render     wgpu + etagere            (core::draw の描画リストを描く)
 └ platform   winit                     (ウィンドウ・入力・イベントループ)
```

| crate | 中身 |
| --- | --- |
| `core` | ゲームからの依頼 (`syscall`)、それを受けて状態を変え描画の指示を組み立てるエンジン (`engine`)、Message Window の中身と文字送り (`content`)、UI Element の木 (`ui`。段 3 までは scene を始めるのに要る分だけ)、文字の整形とラスタライズのインターフェース (`text`)、描画の指示 (`draw`) |
| `vm` | game.wasm を専用の OS スレッドで動かす。import (`biwa:engine` / `biwa:runtime`) を実装し、エンジンに用がある syscall を `core::syscall::Syscall` にして送る |
| `player` | 起動時の引数、ウィンドウを持たない動かし方 (`--headless`) |

### スレッドと syscall

```
  メインスレッド (エンジン)                     wasm のスレッド (vm)
  -------------------------                    ----------------------
  core::Engine                                  wasmtime の Store / game.wasm
     ^  止まらない syscall のまとまり (Batch) ---|
     |  止まる syscall (Call) ------------------|  返事が来るまで recv() で止まる
     |---------- 返事 (CallReply) ------------->|
```

- TypeScript 版の Worker とメインスレッドの分担 (`engine/nodejs/src/engine/vm/wasm/`) と同じ形で、輸送路だけが違う。
  SharedArrayBuffer も `Atomics.wait` も COOP / COEP も要らない。
- 止まらない syscall はまとめて 1 通で送る。止まる syscall の前と遷移の発火の後で区切る
  (発火の原子性。`docs/media-object-model.md`)。
- 文字列・Vec・Map の実体は Rust の値を externref に入れる。JS 版の `local` 区分 (wasm の値だけ触る syscall) は
  vm の中で済み、エンジンには来ない。
- 足りない import は instantiate のときに名前付きのエラーになる (JS 版で要った `Module.imports()` の回避は無い)。

## 動かし方

```sh
cargo build --release -p biwa-player

# ウィンドウを出さずに最後まで進める (クリック待ちは自動で進め、Message Window の文字をログに出す)
./target/release/biwa-player --headless --wasm <game.wasm>
#   --trace         syscall を 1 つずつ出す
#   --collector X   wasmtime の GC の方式 (auto / drc / null)
```

wasm は `biwa build --platform web` などでできる `<project>/.biwa_build/wasm/<package>.wasm` を使う。
`--game-dir <DIR>` (その中の `game.wasm`) でもよく、何も指定しなければ実行ファイルの横の `game/` を探す。
(`biwa build` から native のプレイヤー用の配布物を組み立てる経路はまだ無い)

## 段ごとの状況

### 段 1: wasmtime で今の game.wasm が動くか — 済

test1 の scene が wasmtime 50 で最後まで動く (ウィンドウ無し)。
WasmGC の struct・externref の文字列・funcref のハンドラ・止まる syscall のどれも動く。

- 段 3 までは UI を描かないので、最初の SceneStartButton を自動で押して scene を始める。
- メモ §1 の「`StructRef::new` で初期 `Game` を作る」は、今の ABI (`on_click` が `Game[S]` を作る) では要らない。

### 段 2: wasmtime の GC の性能 — 済

`bench/gc-churn` (文字列と struct を大量に作って捨てる。ホスト呼び出し 約 600 万回、文字列 約 590 万個) を
4 コアの Linux サーバ、リリースビルドで 3 回ずつ。比較に同じ wasm を V8 (node 24) で動かした (`bench/run-v8.mjs`)。

| | wasm の実行 | 最大の間隔 (GC の停止) | 1 ms 超の間隔 | 全体 | 最大 RSS |
| --- | --- | --- | --- | --- | --- |
| wasmtime 50 (既定 = 遅延参照カウント) | 410〜430 ms | 3.8〜4.1 ms | 2〜3 回 | 2.5〜2.9 s | 55 MB |
| wasmtime 50 (null = 回収しない) | 440〜460 ms | 0.1〜0.2 ms | 0 回 | 2.8〜2.9 s | 596 MB |
| V8 (node 24) | 630〜670 ms | 5.2〜6.8 ms | 31〜39 回 | 2.2〜2.4 s | — |

- 「wasm の実行」はホスト呼び出しの間の時間の合計、「全体」はホスト側の処理 (文字列の確保など) も含む。
- 既定の GC は 590 万個の文字列を回収しきって 55 MB に収まり、停止は最大 4 ms で V8 より短い。
- **wasm は描画と別のスレッドで動くので、GC が止まっても画面は止まらない** (止まるのはゲームの進行だけ)。
- メモ §1.1 の懸念 (使い捨ての instance 向けの collector に、長命な instance で負荷を掛ける) は、この負荷では問題にならなかった。
- test1 は compile 約 55 ms、wasm の実行 0.5 ms 未満。
- リリースのバイナリは 13.3 MB (wasmtime + Cranelift、LTO、strip)。

### 段 3: 文字 — 作業中

parley + wgpu で Message Window を描く。日本語の折り返し・禁則・文字送り (グリフの先頭 n 個だけ描く)・断片ごとの色 / 大きさ / 太さ / 速度。
