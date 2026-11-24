# Biwa Engine (Node.js based edition)

ノベルゲームエンジンの基盤実装。アーキテクチャと規約は `README.md` に書いてある。
作業前に必ずそちらを読むこと。ここには作業上の注意だけを置く。

## 注意

- これを `npm run build:runtime` したもの (`dist-runtime/`) が Web runtime で、CI がツールチェインに入れる
  (`.github/workflows/runtime-web.yml`)。`biwa` コマンドにはもう同梱しない。
  ゲームは焼き込まず、`src/game/entry.ts` がページの横の `biwa-game.json` と `game.wasm` を実行時に読む。
  `biwa-game.json` は `cli/src/build.rs` (`biwa build`) と `cli/src/serve.rs` (`biwa dev`) が書く。
- 動作を見るときは `npm run watch:runtime` と `biwa dev --platform web --web-runtime <this>/dist-runtime`。
- 生成物は wasm だけである (TypeScript のターゲットは issue #23 で廃止した。エンジンにも TypeScript 向けの経路は無い)。
  `src/game/entry.ts` の default export (`BiwaGame`) から入り、`src/engine/vm/wasm/` が Worker で走らせる。
  ブロッキング syscall は `Atomics.wait` でそのスレッドを止める。syscall の実装は `src/engine/api/*`。
  syscall を足すときは `wasm/contract.ts` の区分、`wasm/host.ts` の対応表、
  `library/std` の `[[native(arch="wasm")]]` (import 宣言も) を対で変更する。
  約束 (import・property の番号など) を変えたら ABI の版を上げる: コンパイラ `biwac_generator::arch::wasm::ABI_VERSION`、
  `wasm/abi.ts`、ネイティブの `engine/desktop-rust/crates/vm/src/abi.rs` の 3 か所 (`docs/toolchain.md` の ABI)。
  止まらない syscall は `wasm/bridge.ts` がまとめて 1 通で流す。
  Worker はゲームの実行中にイベントループへ帰らないので、
  流す契機はそこで明示している (`contract.ts` の `FLUSH_AFTER_CAST`)。
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
- 音は `src/engine/sound/` が持つ。規約と設計は `docs/sound.md`。
  - **論理状態 (`Mixer.ts`) が正で、Web Audio (`WebAudioOutput.ts`) は写し**である。時間・遷移・ダッキング・削除は
    論理状態で決め、Web Audio にはその結果 (`AudioCommand`) を当てるだけにする。Web Audio の時計で何かを決めないこと
    (ネイティブ `crates/core/src/sound.rs` と同じ規則で二重に書いてある。片方を変えたらもう片方も)。
  - 音も canvas と同じエンジン時計で、`main.ts` の 1 本の Ticker から進める。sync の待ちと畳みにも加わる。
  - 時間で動くパラメータは canvas と共通の `src/engine/automation/Automation.ts`。
  - id は SoundObject・Bus・Effect で空間を分ける (`contract.ts` の `ALLOC_SPACE`)。
- アセットを読む API は必ず `src/engine/api/assets.ts` の `resolveAssetUrl()` を通す。
  `.biwa` が書くパスはゲームのパッケージの `assets/` 基準の相対パスで、
  ページの横の `assets/` にある (`biwa build` がコピーし、`biwa dev` が配る)。
- wasm を動かすにはページが cross-origin isolated である必要がある
  (`biwa dev` の `cli/src/serve.rs` と、配布先の HTTP サーバが COOP/COEP を送る)。外すと `SharedArrayBuffer` が消える。
  ただしエンジンが検査するのは `SharedArrayBuffer` の有無だけである
  (デスクトップ版の WebView の事情。`docs/desktop-build.md` §4)。
- `WebAssembly.Module.imports()` は JavaScriptCore (Safari・WebKitGTK) で投げることがある。
  それに頼る処理には必ず逃げ道を用意する (`wasm/worker.ts` の `declaredImports`)。
- パッケージマネージャは npm。Node.js が要るのはエンジンの開発者と CI だけで、ゲームを作る人には要らない。
- GSAP は採用しない (PixiJS Ticker と混在させると事故が起きやすい)。
