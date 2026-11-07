# Biwa ネイティブプレイヤー (Rust)

Biwa のゲーム (`game.wasm`) を、OS の WebView に頼らずに動かすデスクトップのプレイヤー。
wasm ランタイム (wasmtime) を同梱するので、WasmGC が動くかどうかが利用者の環境に左右されない。
方針とクレートの選定は `~/memo/biwa-native-runtime-rust.md` (以下「メモ」)、
配布の仕組み (ゲームを焼き込まないプレイヤーを CI でビルドしておく) は `docs/biwa-prebuilt-player.md`。

状況: メモ §11 の **段 1 (wasmtime で動かす)〜段 5 (UI) が済み**。今の TypeScript 版のエンジンと同じ機能が揃った
(タイトルなどの Page・Link での遷移・SceneStartButton で scene を始める・Canvas・Message Window)。
段 6 (3 OS の実機) の準備として、`biwa build --platform windows|macos|linux` (既定) がこのプレイヤーで配布物を作り、
Actions (`.github/workflows/player-native.yml`) で 3 OS のプレイヤーをビルドできるようにした。

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
 ├ image      image                     (画像の読み込みとデコード。裏のスレッドで)
 ├ layout     taffy                     (UI のレイアウト。core::layout のトレイトを実装する)
 ├ render     wgpu + etagere            (core::draw の描画リストを描く)
 └ platform   winit (+ objc2 / windows-sys) (ウィンドウ・入力・イベントループ・縦横比の固定)
```

| crate      | 中身                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `core`     | ゲームからの依頼 (`syscall`)、それを受けて状態を変え描画の指示を組み立てるエンジン (`engine`)、Message Window の中身と文字送り (`content`)、canvas オブジェクトと遷移 (`canvas` / `transition`)、画像の番号と読み込みの依頼 (`image`)、UI のレイアウトのインターフェース (`layout`)、UI Element の木 (`ui`。Page の切り替え・Link・SceneStartButton・文字の書式の継承)、文字の整形とラスタライズのインターフェース (`text`)、描画の指示 (`draw`) |
| `vm`       | game.wasm を専用の OS スレッドで動かす。import (`biwa:engine` / `biwa:runtime`) を実装し、エンジンに用がある syscall を `core::syscall::Syscall` にして送る                                                                                                                                                                                                                                                                                      |
| `text`     | 断片の列を 1 つの段落として parley で整形し、グリフを swash でラスタライズする。禁則処理 (`kinsoku.rs`)                                                                                                                                                                                                                                                                                                                                          |
| `image`    | 画像のアセットを裏のスレッドで読んでデコードし、`core::image::DecodedImage` で返す                                                                                                                                                                                                                                                                                                                                                               |
| `layout`   | `core` が作ったレイアウト用の木を taffy (block / flex / grid) で計算する。CSS への写し方は TypeScript 版の DOM と同じ                                                                                                                                                                                                                                                                                                                            |
| `render`   | 描画の指示を wgpu で描く。塗りの矩形・グリフ・画像を「中心・大きさ・回転を持つインスタンスの矩形」として 1 本のパイプラインで描く。グリフはアルファだけのアトラス (etagere) に、画像は 1 枚ずつテクスチャに載せ、テクスチャが替わるところで描画の呼び出しを区切る。切り取りは scissor                                                                                                                                                            |
| `platform` | winit のイベントループを、自前のイベント (大きさの変化・クリック (位置つき)・進む・描き直し・見えるようになった・起こされた・閉じる) にして渡す。全画面の切り替え (F11・Alt+Enter・macOS は Cmd+Ctrl+F) とウィンドウの縦横比の固定 (macOS・Windows) もここ                                                                                                                                                                                                                                                                                                                      |
| `player`   | 起動時の引数、ウィンドウを出す動かし方 (`windowed.rs`)、出さない動かし方 (`--headless`)                                                                                                                                                                                                                                                                                                                                                          |

各 crate が直接依存する外部 crate (`cargo tree --depth 1` で確かめた):

| crate      | 外部 crate                                                             |
| ---------- | ---------------------------------------------------------------------- |
| `core`     | なし                                                                   |
| `vm`       | wasmtime                                                               |
| `text`     | parley, swash (と、ICU の警告を黙らせるための icu_provider の feature) |
| `image`    | image                                                                  |
| `layout`   | taffy                                                                  |
| `render`   | wgpu, etagere, bytemuck, pollster, raw-window-handle                   |
| `platform` | winit, raw-window-handle, objc2 / objc2-foundation (macOS), windows-sys (Windows) |
| `player`   | なし (自前の crate だけ)                                               |

`render` と `platform` の境目は raw-window-handle のトレイト (ウィンドウの handle の受け渡しの標準) だけで、
`player` は名前を出さない。

### スレッドと syscall

```
  メインスレッド (エンジン)                     wasm のスレッド (vm)
  -------------------------                    ----------------------
  core::Engine                                  wasmtime の Store / game.wasm
     ^  止まらない syscall のまとまり (Batch) ---|
     |  止まる syscall (Call) -------------------|  返事が来るまで recv() で止まる
     |---------- 返事 (CallReply) -------------->|
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
cargo build --release -p biwa-runtime

# ウィンドウを出す。Link・SceneStartButton はクリックで押し、それ以外のクリック (か Enter / Space) で進む
./target/release/biwa-runtime --wasm <game.wasm> [--assets <DIR>]

# ウィンドウを出さずに最後まで進める (最初の SceneStartButton を自動で押し、クリック待ちは自動で進め、
# Message Window の文字をログに出す)
./target/release/biwa-runtime --headless --wasm <game.wasm>
#   --trace         syscall を 1 つずつ出す
#   --collector X   wasmtime の GC の方式 (auto / drc / null)
```

wasm は `biwa build --platform web` などでできる `<project>/.biwa_build/wasm/<package>.wasm` を使う。
`--game-dir <DIR>` (その中の `game.wasm`) でもよく、何も指定しなければ実行ファイルの横の `game/` を探す。

アセット (画像) の置き場所は `--assets <DIR>`、無ければ `<game>/assets`。
`--wasm` で `<project>/.biwa_build/wasm/<package>.wasm` を指したときは `<project>/assets` を使う。

確かめ用のゲーム:

- `examples/canvas-check`: canvas の回転の向き・Canvas の範囲での切り取り・レイヤーの前後
  (`biwa build --platform web` で wasm を作ってから `--wasm` で指す)
- `bench/gc-churn`: GC の性能 (段 2)

開発モード (`--dev`。`biwa dev` が付ける): 標準入力の `reload` の行 (と F5) で、ウィンドウを開いたまま
wasm とアセットを読み直してゲームを最初からやり直す。標準入力が閉じたら終わる (`crates/player/src/dev.rs`)。
`--title <TITLE>` でウィンドウのタイトルを決められる。

配布物は `biwa build --platform windows,macos,linux` で作る (`docs/desktop-build.md`)。
`game/biwa-game.json` の `title` がウィンドウのタイトルになる。
Windows のリリースビルドはコンソールを開かないので、起動に失敗したときのエラーは
一時ディレクトリの `biwa-runtime-error.log` にも書く。

## 段ごとの状況

### 段 1: wasmtime で今の game.wasm が動くか — 済

test1 の scene が wasmtime 50 で最後まで動く (ウィンドウ無し)。
WasmGC の struct・externref の文字列・funcref のハンドラ・止まる syscall のどれも動く。

- 段 3 までは UI を描かないので、最初の SceneStartButton を自動で押して scene を始める。
- メモ §1 の「`StructRef::new` で初期 `Game` を作る」は、今の ABI (`on_click` が `Game[S]` を作る) では要らない。

### 段 2: wasmtime の GC の性能 — 済

`bench/gc-churn` (文字列と struct を大量に作って捨てる。ホスト呼び出し 約 600 万回、文字列 約 590 万個) を
4 コアの Linux サーバ、リリースビルドで 3 回ずつ。比較に同じ wasm を V8 (node 24) で動かした (`bench/run-v8.mjs`)。

|                                       | wasm の実行 | 最大の間隔 (GC の停止) | 1 ms 超の間隔 | 全体       | 最大 RSS |
| ------------------------------------- | ----------- | ---------------------- | ------------- | ---------- | -------- |
| wasmtime 50 (既定 = 遅延参照カウント) | 410〜430 ms | 3.8〜4.1 ms            | 2〜3 回       | 2.5〜2.9 s | 55 MB    |
| wasmtime 50 (null = 回収しない)       | 440〜460 ms | 0.1〜0.2 ms            | 0 回          | 2.8〜2.9 s | 596 MB   |
| V8 (node 24)                          | 630〜670 ms | 5.2〜6.8 ms            | 31〜39 回     | 2.2〜2.4 s | —        |

- 「wasm の実行」はホスト呼び出しの間の時間の合計、「全体」はホスト側の処理 (文字列の確保など) も含む。
- 既定の GC は 590 万個の文字列を回収しきって 55 MB に収まり、停止は最大 4 ms で V8 より短い。
- **wasm は描画と別のスレッドで動くので、GC が止まっても画面は止まらない** (止まるのはゲームの進行だけ)。
- メモ §1.1 の懸念 (使い捨ての instance 向けの collector に、長命な instance で負荷を掛ける) は、この負荷では問題にならなかった。
- test1 は compile 約 55 ms、wasm の実行 0.5 ms 未満。
- リリースのバイナリは 13.3 MB (wasmtime + Cranelift、LTO、strip)。

### 段 3: 文字 — 済

test1 の scene の Message Window をウィンドウに描き、クリックで進められる
(この Linux サーバの Xvfb + lavapipe (ソフトウェアの Vulkan) で確認。実機の GPU と HiDPI はまだ)。

- 日本語の整形・折り返し・断片ごとの色 / 大きさ / 太さ / 速度。
- **文字送りは「1 度だけ整形して、グリフの先頭 n 個だけ描く」。** 送りの間は再整形しないので行が動かない
  (TypeScript 版の `visibility: hidden` と同じ保証を、より安く)。クリックはまず送りを畳む。
- 描き直しはオンデマンド。文字送りも sleep も無いときはイベントが来るまで眠る (常時 60 fps で回さない)。
- 禁則: ICU4X (UAX #14) は日本語のロケールのとき、小書きの仮名・長音符・句読点の前ではもともと改行しない (実測)。
  取りこぼし (全角のチルダ「～」など) を `kinsoku.rs` の表で拾う。
- フォントは今はシステムの sans-serif (日本語のロケール)。太字の無いフォントでは輪郭を太らせる (疑似ボールド)。
  parley の run が疑似ボールドを伝えないことがあったので、要求した太さと選ばれたフォントの太さから自分で決めている。
- ICU4X は日本語の分かち書きの辞書が無いと整形のたびに警告を出す (改行位置には影響しない)。
  `icu_provider` の `logging` feature で `log` crate 経由にして黙らせている。辞書 (parley の `complex-scripts`) は数 MB あるので入れていない。

### 段 4: canvas — 済

test1 の scene の背景・立ち絵の出入り・揺れながらの移動・消え方を描き、`examples/canvas-check` で
回転の向き・切り取り・レイヤーを確かめた (Xvfb + lavapipe)。

- `CanvasObjects.ts` の規則をそのまま移した (`core::canvas`):
  biwa 側のパラメータを正として毎フレーム射影する、積まれた `(オブジェクト, パラメータ)` だけを置き換える、
  次の区間が前の区間の値を引き継ぐ、周期系は偏差だけを乗せる、sync 印の束をクリックで終端まで飛ばす
  (時計ではなく区間の開始時刻をずらす)、大きさを指定しない w / h を画像の元の大きさから 1 度だけ決める。
- 区間は番号で持つ表に置き、走っている列と sync の束は番号で指す (TypeScript 版は同じオブジェクトを両方から参照していた)。
  どこからも指されなくなった区間は毎フレーム捨てる。
- 演出待ち (`sys_await_transitions`) は sync の束が終わるまで返事を保留する。クリック待ちの途中のクリックは、
  文字送りと sync の演出を両方畳む (短絡させない)。
- 画像はエンジンが番号を振って読み込みを依頼するだけで (`Engine::take_image_requests`)、
  `image` crate が裏のスレッドで読み、player がエンジン (元の大きさ) と `render` (テクスチャ) に渡す。
  同じパスは 1 度だけ読む。読み込みが終わるまでそのオブジェクトは描かない (TypeScript 版と同じ)。
- パスは `assets/` 基準の相対パスで、`..` や絶対パスは拒む (`api/assets.ts` と同じ)。
- Canvas の外にはみ出した分は描かない (scissor)。レイヤーは番号順、同じレイヤーの中は作った順。

### 段 5: UI — 済

test1 のタイトル (Box の文字・Image・Link)、2 ページ目 (Page の背景画像・`Layers` で重ねた画像・2 つのボタン)、
SceneStartButton のクリックで scene に入るところまでを描いて確かめた (Xvfb + lavapipe)。
WebKit 版のスクリーンショットと並べて、タイトル画面の位置が数 px 以内で一致した。

- UI の木 (`core::ui`) は `UIObjects.ts` の規則をそのまま移した:
  Window は作った時点で出る、親が子をいくつ持てるか (多数 / 1 つ / 持てない) と、それによる Element の破棄・預けた関数の手放し、
  最初に積まれた Page が見える、Link で page_id の Page に替わる、ScenePage は scene が始まるときに見える、
  Window は Page を積む前に ScenePage と `main_scene` を持っていなければならない、Canvas / MessageArea を 2 か所に置けない、
  `id` の逆引き、文字の書式 (大きさ・太さ・色・フォント) の親からの継承 (既定は 16 px・400・黒・sans-serif)。
- レイアウトは、`core` が UI の木を素朴な木 (`core::layout`: 並べ方・大きさ・余白・文字) にし、`layout` crate が taffy で計算する。
  CSS への写し方は TypeScript 版の DOM と同じ (表は `crates/layout/src/lib.rs`)。
  Element の `text` は子より前に置いた「文字だけの葉」にして、整形 (`core::text::TextShaper`) で大きさを測る。
- 描画は、レイアウトの木の順に、背景 (色 / 画像は `cover`)・Image Element・Canvas (の中に canvas オブジェクト)・
  MessageArea (の中に Message Window)・文字を積む。重なりの順は木の順 (= `Layers` の push 順)。
- レイアウトは UI が変わったときか画面の大きさが変わったときだけ計算し直す (UI の木の世代番号で見分ける)。
- クリックは、最後に描いたフレームの押せる Element (Link・SceneStartButton) に当たり判定する。
  当たれば押し (Link は Page を替え、SceneStartButton は vm に scene を始めさせる)、当たらなければ「進む」。
  Link・ボタンのクリックは「進む」には使わない (TypeScript 版でクリックを host に伝えないのと同じ)。
- UI の文字の行の高さは CSS の既定と同じく、フォント自身の値 (`line-height: normal`) で決める。
  Message Window は TypeScript 版と同じく文字の大きさの 1.8 倍。
- 疑似ボールドを少し太くした (文字の大きさの 1/16、0.6〜2.5 px)。Chromium の太字に近づけるため。

段 5 での違い (TypeScript 版の DOM とずれうるところ):

- Link / SceneStartButton は CSS では inline-flex (行の中に横に並ぶ) だが、taffy には行の中の配置が無いので、
  block の親の中では 1 行を占める (flex / grid の親の中では同じ)。幅を指定しないと親の幅いっぱいに伸びる。
- マウスを載せたときのカーソルの形 (pointer) は変えていない。
- 中身があふれたときのスクロール (`overflow-y: auto`) は無い。
- `TextFont` はフォント名として fontique に渡す (CSS の font-family と同じくカンマ区切りの候補)。見つからなければ既定のフォント (Windows は Yu Gothic・Meiryo、macOS は Hiragino Sans、Linux は sans-serif)。

### 段 6: 3 OS の実機 — 準備済み (実機での確認待ち)

- `biwa build` の `--platform windows|macos|linux` (と既定の `desktop`) をこのプレイヤーに切り替えた。
  Tauri 版は `--platform windows-webview|macos-webview|linux-webview`。
  プレイヤーの置き場所の id は `windows-x86_64` / `macos-universal` / `linux-x86_64` (Tauri 版は `*-webview-*`)。
- ゲームのデータは `game.wasm` + `assets/` + `biwa-game.json` だけ (Vite は走らない)。
- Actions: `.github/workflows/player-native.yml` (いまは `runtime-native.yml`) が 3 OS でビルドし、`biwa-player-<id>` (いまは `biwa-runtime-native-<os>-<arch>`) として上げる
  (macOS は aarch64 と x86_64 を `lipo` で universal に)。Linux のビルドには `libfontconfig1-dev` が要る。
- Linux (Xvfb + lavapipe) で、組み立てた tar.gz を展開して起動できることを確認済み。
- 実機: Linux (Intel Iris Xe, Vulkan) は最後まで動いた。Windows は CI のテストで文字の幅の違いが出た → 既定のフォントを OS ごとに。
- macOS 実機 (2026-11-06): 動いたが、開いた直後のウィンドウが真っ白で、クリックするまで描かれなかった。
  - 原因: macOS の wgpu は、ウィンドウが見えていない間 (`occlusionState` が Visible でない間) は描き先を渡さず
    `Occluded` を返す (wgpu#8309 の回避。`wgpu-hal` の `metal/surface.rs` の `acquire_texture`)。
    作った直後のウィンドウはまだ画面に出ておらず、最初のフレームは描かれずに捨てられる。
    描き直しはオンデマンドなので、次に描くのはクリックのとき。Linux にはこの判定が無いので起きない。
  - 直し方: `render` が「見えていないので描かなかった」(`FrameStatus::Hidden`) を返し、
    `platform` が winit の `Occluded(false)` (macOS の `windowDidChangeOcclusionState`) を `Event::Shown` にして渡す。
    player は描けていない変更があれば、見えるようになったところで描き直す。
- 縦横比の固定 (2026-11-06):
  - ゲームは決まった大きさの画面 (`biwa-game.json` の `width` / `height`、無ければ 1280x720) として組み、
    ウィンドウに縦横比を保って収める。余りは黒い帯 (レターボックス)。全画面でモニタの比が違っても崩れない。
    帯の上のクリックはどの要素にも当たらないクリック (= 進む) とする。
  - ウィンドウ自体の縦横比も固定する。winit 0.30 に API が無いので OS の API を直接呼ぶ (`platform/src/aspect.rs`)。
    macOS は `NSWindow.contentAspectRatio`、Windows はサブクラス化して `WM_SIZING` で矩形を直す。
    Linux (X11 / Wayland) はできない (帯で保つ)。
  - 全画面の切り替え: F11・Alt+Enter (macOS は Cmd+Ctrl+F と緑のボタンも)。
- 実機の再確認 (2026-11-07):
  - 縮めて (拡大率 0.4 前後) 帯を出すと、Link の文字が箱の下にはみ出した。
    原因: レイアウトは文字を論理ピクセルの大きさで測り、描くときは物理ピクセルの大きさで整形し直していた。
    行の高さはその大きさで丸められるので、文字が小さいほど測った高さと描いた高さの差が大きくなる。
    → レイアウトも描くときと同じ大きさ (物理ピクセル) で測る (`layout` の measure。キャッシュの鍵に拡大率を足した)。
    描くときの折り返し幅の余裕は 1 論理ピクセル分にし、中央寄せはその半分だけ戻す。
  - macOS でウィンドウの縦横比が固定されなかった。原因: winit は角や辺をつまむたび (`windowWillStartLiveResize:`) に
    `contentResizeIncrements` を設定し直し、AppKit ではそれと両立しない `contentAspectRatio` が消える。
    → 大きさが変わるたびに縦横比を付け直す。
  - Windows で起動時に左上から白い矩形が見え、しばらくして描かれた。最初のフレームを描く前のウィンドウの中身が見えている。
    → Windows はウィンドウを隠したまま作り、最初のフレーム (準備中なら黒) を描いてから見せる。
    リリースビルドでもコマンドラインから起動すれば、そのコンソールに `[biwa]` のログが出るようにした (`AttachConsole`)。
- 実機の再々確認 (2026-11-07): Windows の白い矩形と macOS の縦横比は解消。Link の文字はほぼ収まったが、
  縮めると Message Window の行間が縮まず、文字が下にはみ出した。
  - 原因: parley は段落の既定の文字の大きさ (渡さなければ 16 物理ピクセル) を行の高さの下限にする (CSS の strut と同じ)。
    縮めて描くと文字はそれより小さくなるので、行の高さが 16 × 1.8 物理ピクセルで止まっていた。
  - 直し方: `ParagraphStyle::base_size` (段落の文字の大きさ、物理ピクセル) を足し、エンジンは CSS の既定 16 論理ピクセル
    (`text::DEFAULT_FONT_SIZE`) × 拡大率を渡す。等倍では TypeScript 版 (要素の font-size はブラウザの既定のまま) と同じで、
    縮めても同じ比で縮む。

実行ファイルの名前は `biwa-runtime` (2026-11-07 に `biwa-player` から改名。crate のディレクトリ `crates/player` と
部品の crate 名 `biwa-player-*` はまだ元のまま)。

## まだ無いもの

- Windows / macOS 実機での縦横比の固定と全画面の確認 (段 6)、サウンド・IME・ルビ・動画 (段 7〜10)。
- アニメーションする GIF / WebP、画像の mipmap (大きく縮めるとちらつきうる)。
- ゲームに同梱したフォントの登録と、`TextFont` property。
- 実機 (Windows / macOS / Linux の GPU、HiDPI、Wayland) での確認。
