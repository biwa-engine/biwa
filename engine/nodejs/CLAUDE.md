# Biwa Engine (Node.js based edition)

ノベルゲームエンジンの基盤実装。アーキテクチャと規約は `README.md` に書いてある。
作業前に必ずそちらを読むこと。ここには作業上の注意だけを置く。

## 注意

- これを `npm run build:runtime` したもの (`dist-runtime/`) が Web runtime で、CI がツールチェインに入れる
  (`.github/workflows/runtime-web.yml`)。`biwa` コマンドにはもう同梱しない。
  ゲームは焼き込まず、`src/game/entry.ts` がページの横の `biwa-game.json` と `game.wasm` を実行時に読む。
  `biwa-game.json` は `cli/src/build.rs` (`biwa build`) と `cli/src/serve.rs` (`biwa dev`) が書く。
- 動作を見るときは `npm run watch:runtime` と `biwa dev --platform web --web-runtime <this>/dist-runtime`。
- 生成物は wasm と TypeScript の 2 通りあったが、TypeScript (deprecated) は読み込む経路が無くなった。
  エンジンの中の TypeScript 向けの部分 (kernel など) は残っているだけである。
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
- Message Window に出るものは `src/components/TextBox.ts` が持つ。
  規約と設計は `docs/content-api.md`。
  - **エンジンは設定を持たない。** 速度・大きさ・色は `Game` の `Config` にあり、
    std が絶対値に潰してから syscall で渡す。エンジン側に既定値を置かないこと。
  - **エンジンは自分の判断で枠をクリアしない。** `sys_content_clear` が来たときだけ消す。
    いつ消えるかを決めるのは std である (「待つが消さない」API を足すときに
    変更がエンジンまで波及しないようにするため)。
  - 文字送りの途中の文字は `visibility: hidden` で隠す。`textContent` を
    伸ばす形にすると折り返しが変わって行がずれる。
- canvas に置くものは `src/engine/canvas/CanvasObjects.ts` が持つ。
  biwa 側のパラメータを正とし、PixiJS へは毎フレーム射影する
  (座標系も単位も両者で違う: 中央原点・y は上が正・alpha は 0-255・theta は度)。
  規約と設計は `docs/media-object-model.md`。
  - **Ticker に登録するコールバックを増やさないこと。** `main.ts` の 1 つだけである
    (`CanvasObjects` も `TextBox` もそこから駆動されている)。
    オブジェクトごとに生やすとリークするし、
    ポーズ・オート・スキップが 1 箇所で効かなくなる。
  - 時間は `performance.now()` ではなくエンジン時計 (Ticker の差分の累積) で測る。
  - param / kind の番号は `src/engine/api/transition.ts` にある。
    wasm の std はこれを `.wat` に写しているので、変えるなら
    `library/std` の import 宣言と合わせて動かす。
- アセットを読む API は必ず `src/engine/api/assets.ts` の `resolveAssetUrl()` を通す。
  `.biwa` が書くパスはゲームのパッケージの `assets/` 基準の相対パスで、
  ページの横の `assets/` にある (`biwa build` がコピーし、`biwa dev` が配る)。
- wasm を動かすにはページが cross-origin isolated である必要がある
  (`biwa dev` の `cli/src/serve.rs` と、配布先の HTTP サーバが COOP/COEP を送る)。外すと `SharedArrayBuffer` が消える。
  ただしエンジンが検査するのは `SharedArrayBuffer` の有無だけである
  (デスクトップ版の WebView の事情。`docs/desktop-build.md` §4)。
- `WebAssembly.Module.imports()` は JavaScriptCore (Safari・WebKitGTK) で投げることがある。
  それに頼る処理には必ず逃げ道を用意する (`wasm/worker.ts` の `declaredImports`)。
- std の TypeScript 向けの native が書く `@biwa/engine/<path>` という論理パスは、TypeScript ターゲットとともに
  使われなくなった (alias は外した)。
- パッケージマネージャは npm。Node.js が要るのはエンジンの開発者と CI だけで、ゲームを作る人には要らない。
- `tsconfig.json` では未使用シンボルの検査を有効にしない (TypeScript ターゲットの名残り)。
- GSAP は採用しない (PixiJS Ticker と混在させると事故が起きやすい)。
- wasm ターゲットが tier 1、TypeScript ターゲットが tier 2。判断が割れたら wasm を優先する。
