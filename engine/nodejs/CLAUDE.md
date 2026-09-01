# Biwa Engine (Node.js based edition)

ノベルゲームエンジンの基盤実装。アーキテクチャと規約は `README.md` に書いてある。
作業前に必ずそちらを読むこと。ここには作業上の注意だけを置く。

## 注意

- このディレクトリは `biwa` コマンドに丸ごと同梱され、
  `biwa dev` 実行時にプロジェクトの `.biwa_runtime/` へ展開される。
  ファイルを増やすとそのまま同梱物が増える (`cli/src/assets.rs` の exclude を参照)。
- `src/game/` は `biwa dev` が生成物で上書きする領域。
  リポジトリにある `entry.ts` はエンジン単体で起動するためのプレースホルダ。
- 生成物は wasm と TypeScript の 2 通りある (`biwa dev --target`)。
  どちらで来ても `src/game/entry.ts` の default export (`BiwaBackend`) から入り、
  syscall の実装 (`src/engine/api/*`) は共有する。違うのは輸送路だけである。
  - wasm: `src/engine/vm/wasm/`。Worker で走り、ブロッキング syscall は
    `Atomics.wait` でそのスレッドを止める。
    syscall を足すときは `wasm/contract.ts` の区分、`wasm/host.ts` の対応表、
    `library/std` の `[[native(arch="wasm")]]` (import 宣言も) を対で変更する。
    止まらない syscall は `wasm/bridge.ts` がまとめて 1 通で流す。
    Worker はゲームの実行中にイベントループへ帰らないので、
    流す契機はそこで明示している (`contract.ts` の `FLUSH_AFTER_CAST`)。
  - TypeScript: scene が generator function として出力される。
    `src/engine/vm/` の kernel が `next()` で駆動し、`yield` された syscall を
    処理して結果を書き戻す。
    syscall を足すときは `vm/syscall.ts` の番号、`vm/handlers.ts` の対応表、
    `library/std` の `[[native(arch="typescript")]]` を対で変更する。
    ただし **kernel を通るのは中断する syscall だけ**で、
    中断しないものは std の native が `@biwa/engine/api/*` を直接呼ぶ。
    また、コンパイラが `yield` を置くのは novel statement の展開先だけなので、
    **任意の呼び出しを中断させることは今できない**
    (`await_transitions` / `sleep` が wasm 専用なのはこのため)。
- canvas に置くものは `src/engine/canvas/CanvasObjects.ts` が持つ。
  biwa 側のパラメータを正とし、PixiJS へは毎フレーム射影する
  (座標系も単位も両者で違う: 中央原点・y は上が正・alpha は 0-255・theta は度)。
  規約と設計は `docs/media-object-model.md`。
  - **Ticker に登録するコールバックを増やさないこと。** `main.ts` の 1 つだけである。
    オブジェクトごとに生やすとリークするし、
    ポーズ・オート・スキップが 1 箇所で効かなくなる。
  - 時間は `performance.now()` ではなくエンジン時計 (Ticker の差分の累積) で測る。
  - param / kind の番号は `src/engine/api/transition.ts` にある。
    wasm の std はこれを `.wat` に写しているので、変えるなら
    `library/std` の import 宣言と合わせて動かす。
- アセットを読む API は必ず `src/engine/api/assets.ts` の `resolveAssetUrl()` を通す。
  `.biwa` が書くパスはゲームのパッケージの `assets/` 基準の相対パスで、
  `biwa dev` がそれを `.biwa_runtime/public/assets` にリンクしている。
- wasm を動かすにはページが cross-origin isolated である必要がある
  (`vite.config.ts` が COOP/COEP を送っている)。外すと `SharedArrayBuffer` が消える。
- std から参照される import は `@biwa/engine/<path>` という論理パスで書く規約
  (alias で `src/engine/` に解決される)。
- パッケージマネージャは npm。利用者の環境に Node.js しか仮定しない。
- `tsconfig.json` では未使用シンボルの検査を有効にしない。
  生成された TypeScript が `src/game/` に同居するため。
- GSAP は採用しない (PixiJS Ticker と混在させると事故が起きやすい)。
- wasm ターゲットが tier 1、TypeScript ターゲットが tier 2。判断が割れたら wasm を優先する。
