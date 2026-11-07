# デスクトップ版のビルド (ビルド済み runtime 方式) と実機の検証

ゲームを Windows / macOS / Linux のデスクトップアプリとしてビルドする仕組みと手順、
各 OS で Biwa が動くかを確かめる手順のメモ。

runtime は 2 種類ある (2026-11-02 から)。

| `--platform`                                          | runtime                                                 | `game/` の中身          |
| ----------------------------------------------------- | ------------------------------------------------------- | ----------------------- |
| `windows` / `macos` / `linux` (既定の `desktop`)      | **ネイティブ** (`engine/desktop-rust`。wasmtime + wgpu) | `game.wasm` + `assets/` |
| `windows-webview` / `macos-webview` / `linux-webview` | WebView (`engine/desktop-tauri`。Tauri)                 | Web の静的ビルド (Vite) |

ネイティブの runtime の中身と段階ごとの記録は `engine/desktop-rust/README.md` にある。
この文書の §2 以降の WebView の話 (検証ページ・独自スキーム・localhost) は WebView 版だけのものである。
方針は `docs/biwa-distribution-targets.md` (なぜ WebView か) と
`docs/biwa-prebuilt-player.md` (なぜビルド済み runtime か) にある。

状況 (ネイティブ):

- **Linux**: Xvfb + lavapipe の上で、`biwa build --platform linux` の tar.gz を展開して起動し、
  タイトル (ウィンドウのタイトルはパッケージ名) が出ることを確認済み (2026-11-02)。
- **Linux 実機** (Intel Iris Xe, Vulkan): CI でビルドした runtime の配布物で、シーンの最後まで動いた (2026-11-02)。
- **CI**: 3 OS のビルドに成功。Windows だけ文字の幅のテストが落ちた
  (既定の sans-serif が Arial で、日本語が仮名の詰まった Yu Gothic UI に落ちていた) →
  既定のフォントを Windows は `Yu Gothic, Meiryo`、macOS は `Hiragino Sans` からにした。
- **macOS 実機** (2026-11-06): 動いたが、開いた直後が真っ白でクリックまで描かれなかった
  (wgpu が見えていないウィンドウには描かないため。`engine/desktop-rust/README.md` の段 6) → 直した。再確認待ち。
- **実機の再確認** (2026-11-07): macOS は白い画面が直り、中身の 16:9 と全画面を確認。Windows はウィンドウの縦横比の固定と
  全画面を確認。Linux は中身の 16:9 と全画面を確認。残った 3 件 (縮めたときの Link の文字のはみ出し、macOS のウィンドウの
  縦横比、Windows の起動時の白い矩形) を直した (`engine/desktop-rust/README.md` の段 6)。
- **再々確認** (2026-11-07): Windows の白い矩形と macOS の縦横比は解消。縮めたときに Message Window の行間が縮まない件を直した。
- Windows のリリースビルドは PowerShell などから起動すると、そのコンソールに `[biwa]` のログ (使っている GPU など) が出る。

状況 (WebView):

- **Linux (Ubuntu 24.04, WebKitGTK 2.52)**: runtime のビルド、配布物の組み立て、起動、タイトルからシーンまでの進行を確認済み。
- **CI**: 3 OS の runtime のビルドに成功 (2026-10-13。当時は `player.yml`、いまは `runtime-webview.yml`)。
- **Windows / macOS**: CI でビルドした runtime から、Linux の上で配布物を組み立てた
  (macOS は universal バイナリの両方のアーキテクチャに ad-hoc 署名が付くことを確認)。
  - Windows: 実機で動いた (SmartScreen の警告は出る)。§4
  - macOS: 最初は動かなかった (WebKit が古く WasmGC が無い、独自スキームで SharedArrayBuffer が出ない)。
    Safari を更新し (macOS 27 / Safari 22625)、localhost で配れば検証ページの必須項目はすべて OK になった。
    そこで macOS では localhost で配るのを既定にした (2026-10-26。この runtime での実機確認はまだ)。§4
- CI でビルドした Linux の runtime から組み立てた配布物も、Linux で起動しシーンまで進むことを確認済み。

## 1. 構成

```
biwa build                              → dist/<host OS>/   (既定。--platform desktop と同じ。ネイティブ)
biwa build --platform windows,macos,linux → dist/windows/ dist/macos/ dist/linux/
biwa build --platform windows-webview,macos-webview,linux-webview
                                        → dist/windows-webview/ dist/macos-webview/ dist/linux-webview/
biwa build --platform web               → dist/web/
```

### ネイティブの runtime (`engine/desktop-rust/`)

- ゲームを焼き込まない汎用の実行ファイル (`biwa-runtime`)。**Biwa の CI が OS ごとに 1 度だけビルドする**
  (`.github/workflows/runtime-native.yml`)。
- 実行ファイルの横の `game/` (macOS は `<name>.app/Contents/Resources/game/`) の `game.wasm` を読み、
  wasmtime で動かす。画像などは `game/assets/` から読む。`--game-dir <DIR>` / `--wasm <FILE>` で場所を上書きできる。
- `game/biwa-game.json` の `title` をウィンドウのタイトルにする (無ければ `Biwa`)。
- ゲームは決まった大きさの画面 (`biwa-game.json` の `width` / `height`、無ければ 1280x720) として組み、
  ウィンドウに縦横比を保って収める (余りは黒い帯)。ウィンドウの縦横比も macOS・Windows では固定する
  (Linux は帯だけ)。全画面の切り替えは F11・Alt+Enter (macOS は Cmd+Ctrl+F と緑のボタンも)。
- 描画は wgpu (Windows: DX12 / Vulkan、macOS: Metal、Linux: Vulkan / GL)。
- Windows のリリースビルドはコンソールを開かない。起動に失敗したときのエラーは
  一時ディレクトリの `biwa-runtime-error.log` (Windows なら `%TEMP%\biwa-runtime-error.log`) にも書く。
- 実行ファイルの大きさ: Linux で約 22 MB (wasmtime の Cranelift を含むため)。

### WebView の runtime (`engine/desktop-tauri/`)

- ゲームを焼き込まない汎用の実行ファイル (Tauri v2)。**Biwa の CI が OS ごとに 1 度だけビルドする**
  (`.github/workflows/runtime-webview.yml`)。利用者の手元で Rust のビルドは走らない。
  - デスクトップ向けの実装は今後増えうるので、ディレクトリ名は実装 (Tauri) に結びつけている。
- 実行ファイルの横の `game/` (macOS は `<name>.app/Contents/Resources/game/`) を配る。
  `--game-dir <DIR>` (または `BIWA_GAME_DIR`) で場所を上書きできる。配り方は OS で違う。
  - Windows / Linux: 独自スキーム `biwa://localhost/` (Windows は `http://biwa.localhost/`)。
  - macOS: runtime の中の HTTP サーバで `http://localhost:<port>/`。
    WKWebView は独自スキームのページで `SharedArrayBuffer` を出さないため (§4)。
    サーバは 127.0.0.1 だけで待ち受ける。
    ポートはパッケージ名から決まる値 (40000〜48999) で、塞がっていれば次を試し、だめなら空いているもの。
    オリジンが起動のたびに変わると、WebView のストレージ (localStorage など) が前回のものを読めなくなるため。
  - 配るときに COOP / COEP / CORP のヘッダを付ける (`src-tauri/src/serve.rs`)。
  - 音や動画のシークのために `Range` に答える。
  - `game/` の外のパスは拒む。
- `game/biwa-game.json` (`biwa build` が書く) からウィンドウのタイトルなどを読む。
- `game/` が無ければ、案内のページ (`placeholder/`) を出す。
- 実行ファイルの大きさ: Linux で約 3.6 MB。

### `biwa build` (`cli/src/build.rs`, `desktop.rs`, `toolchain.rs`)

ネイティブ版:

1. コンパイルし、wasm (`.biwa_runtime/game/game.wasm`) とパッケージの `assets/`、`biwa-game.json` を
   ゲームのデータにする (`.biwa_runtime/dist-game-native/`)。Node.js (Vite) は使わない。
2. OS ごとに、ビルド済みの runtime にゲームのデータを添えて組み立て、固める (下の表)。

WebView 版:

1. コンパイルし、Web の静的ビルド (`vite build`) を作る。
   - `base` は `./`。Vite が出すもの (JS・wasm・Worker) は `_biwa/` に置く
     (`engine/nodejs/vite.config.ts` の `build.assetsDir`。既定の `assets/` はパッケージの `assets/` と重なるため)。
   - パッケージの `assets/` は出力へコピーする (開発時のようなリンクにはしない)。
2. デスクトップ版では、それに `biwa-game.json` と検証ページ (`__biwa_probe/`、ゲームの wasm の写し付き) を足して
   ゲームのデータにする (`.biwa_runtime/dist-game/`)。
3. OS ごとに、ビルド済みの runtime にゲームのデータを添えて組み立て、固める。

| OS      | `dist/<os>/` にできるもの                                                      |
| ------- | ------------------------------------------------------------------------------ |
| Windows | `<name>/<name>.exe` + `<name>/game/`、それを固めた `<name>-windows-x86_64.zip` |
| macOS   | `<name>.app` (ad-hoc 署名済み)、それを固めた `<name>-macos-universal.zip`      |
| Linux   | `<name>/<name>` + `<name>/game/`、それを固めた `<name>-linux-x86_64.tar.gz`    |

WebView 版は `dist/<os>-webview/` に、`<name>-<os>-webview-<arch>.{zip,tar.gz}` という名前でできる。

- **組み立てにコンパイルは要らないので、どの OS の上でも全 OS 向けに作れる。**
- macOS の .app はデータを足すと署名が合わなくなるので、組み立てたあとで `apple-codesign` (純 Rust) で
  ad-hoc 署名し直す (Apple Silicon で起動するのに最低限要る)。
  Info.plist・リソース (`Resources/game/` を含む) のダイジェストが署名に入る。
- 固めるのは、実行ビットと .app の構造を確実に残すため (CI の成果物やコピーの途中で実行ビットは落ちやすい)。

### 配布物の中身 (`game/` の構成)

ネイティブ版:

```
<name>/                       (macOS は <name>.app/Contents/)
  <name>[.exe]                ← runtime (biwa-runtime を改名したもの。macOS は MacOS/<name>)
  game/                       (macOS は Resources/game/)
    game.wasm                 ← ゲーム (必須)
    assets/                   ← パッケージの assets/ のコピー
    biwa-game.json            ← runtime が読むゲームの情報 (無くても動く。いまは title だけ読む)
```

WebView 版: `game/` は Web の静的ビルド (`biwa build --platform web` の `dist/web/` と同じもの) に、
runtime 用の 2 つを足しただけである。

```
<name>/                       (macOS は <name>.app/Contents/)
  <name>[.exe]                ← runtime (biwa-runtime を改名したもの。macOS は MacOS/<name>)
  game/                       (macOS は Resources/game/)
    index.html                ← runtime が最初に開くページ (必須)
    _biwa/                    ← エンジンの JS・Worker・ゲームの wasm (Vite の出力。名前にハッシュが付く)
    assets/                   ← パッケージの assets/ のコピー
    favicon.svg ほか           ← エンジンの public/ の中身
    biwa-game.json            ← runtime が読むゲームの情報 (無くても動く)
    __biwa_probe/             ← WebView の検証ページ (--probe で開く。無くても動く)
      index.html, probe.js, worker.js, game.wasm
```

`biwa-game.json` の中身 (どのキーも省略できる。無ければタイトルは `Biwa`、大きさは 1280x720):

```json
{
  "name": "test1",
  "title": "test1",
  "version": "0.1.0",
  "width": 1280,
  "height": 720
}
```

(`name` と `version` はいま runtime は読まない。`width` / `height` は WebView 版はウィンドウの大きさ、ネイティブ版はゲームの画面の大きさ (縦横比も) になる。`biwa build` はまだ書かない)

#### 手で組み立てる

`biwa build` を通さずに同じ状態を作れる (runtime の動作の切り分けに使う)。Linux の例:

```sh
mkdir -p mygame
cp ~/.biwa/toolchains/<toolchain>/runtime/webview-linux-x86_64/biwa-runtime mygame/mygame
chmod +x mygame/mygame
biwa build --platform web               # <project>/dist/web/ ができる
cp -r <project>/dist/web mygame/game
echo '{ "title": "mygame" }' > mygame/game/biwa-game.json   # 任意
./mygame/mygame
```

- `game/index.html` が無いと、runtime は「`game/` が見つからない」案内のページを出す (標準エラーにも出る)。
- `game/` を別の場所に置いたまま試すなら `--game-dir <DIR>` (または `BIWA_GAME_DIR`)。
  例: `~/.biwa/toolchains/<toolchain>/runtime/webview-linux-x86_64/biwa-runtime --game-dir <project>/dist/web`
- 検証ページも使うなら、`<project>/.biwa_runtime/dist-game/__biwa_probe/` (デスクトップ版の `biwa build` が作る) を
  `game/__biwa_probe/` にコピーする。
- macOS は .app の構造 (`Contents/{Info.plist, MacOS/<name>, Resources/game/}`) と署名が要るので、
  手で組み立てるより `biwa build --platform macos` を使う方がよい
  (手で触ったら `codesign --force --deep -s - <name>.app` で ad-hoc 署名し直す)。

### ツールチェインと runtime の置き場所

CLI と runtime は「ツールチェイン」という単位で揃えて使う (コンパイラが出す wasm と runtime の間に約束があるため)。
バージョン番号がまだ無いので、ツールチェインの名前はビルドしたコミットの先頭 12 文字 (`biwa --version` に出る。
手元で未コミットの変更からビルドした CLI は `-dirty` が付く)。

```
<home>/                                   ← 環境変数 BIWA_HOME、無ければ ~/.biwa
  bin/biwa[.exe]                          ← CLI (いまはこれが実体。いずれランチャーにする)
  toolchains/<toolchain>/runtime/
    native-windows-x86_64/biwa-runtime.exe    ← ネイティブ (runtime-native.yml)
    native-macos-universal/biwa-runtime
    native-linux-x86_64/biwa-runtime
    webview-windows-x86_64/biwa-runtime.exe   ← WebView (runtime-webview.yml)
    webview-macos-universal/biwa-runtime
    webview-linux-x86_64/biwa-runtime
```

`biwa build` は CLI と同じ名前のツールチェインの runtime を使う。実験のために別のものを使うときは
`--toolchain <NAME>` (または環境変数 `BIWA_TOOLCHAIN`)。

#### 入手する (Actions の release ワークフロー)

`.github/workflows/release.yml` を手で実行すると (workflow_dispatch)、同じコミットから CLI 3 OS 分と runtime 6 種類を
ビルドし、ホストの OS ごとに `~/.biwa` にそのまま展開できるアーカイブを作る
(成果物 `biwa-toolchain-<host>`、中身は `biwa-<toolchain>-<host>.{tar.gz,zip}`)。
どのアーカイブにも全ターゲットの runtime が入っているので、どの OS の上でも全 OS 向けの配布物を作れる。

```sh
# macOS / Linux (host は macos-universal か linux-x86_64)
gh run download <run-id> -R biwa-engine/biwa -n biwa-toolchain-<host> -D /tmp/biwa
mkdir -p ~/.biwa && tar -xzf /tmp/biwa/biwa-*-<host>.tar.gz -C ~/.biwa
export PATH="$HOME/.biwa/bin:$PATH"     # シェルの設定に書く
biwa --version
```

```powershell
# Windows (PowerShell)
gh run download <run-id> -R biwa-engine/biwa -n biwa-toolchain-windows-x86_64 -D $env:TEMP\biwa
Expand-Archive $env:TEMP\biwa\biwa-*-windows-x86_64.zip -DestinationPath $env:USERPROFILE\.biwa -Force
$env:Path = "$env:USERPROFILE\.biwa\bin;$env:Path"    # 恒久的にはユーザの環境変数 Path に足す
biwa --version
```

展開は上書きで、`toolchains/` の下はツールチェインごとに分かれるので、前のものは残る (`bin/biwa` だけが入れ替わる)。
macOS で curl / gh で取ったファイルには quarantine が付かないので、Gatekeeper には止められない。

runtime だけの workflow (`runtime-native.yml` / `runtime-webview.yml`) も push で走る。その成果物
`biwa-runtime-<runtime-id>` を手で `toolchains/<name>/runtime/<runtime-id>/` に置き、`--toolchain <name>` で使ってもよい
(手元でビルドした CLI で試すとき。CLI とコミットがずれるので、組み合わせが合うかは自分で確かめる)。

将来は配布サーバ (GitHub Releases) とインストールスクリプト、`biwa` 自身による更新にする
(`~/memo/biwa-toolchain-versioning-and-release.md`)。

## 2. 準備

### ゲームをビルドする人

- `biwa` (リポジトリの `cli/` で `cargo build --release`。いまは手元でビルドする)。
- Node.js (npm 付き)。WebView 版と Web 版の静的ビルドに使う (ネイティブ版だけなら要らない)。
- ビルド済みの runtime (上記)。
- **Rust も C++ Build Tools も Xcode も要らない** (runtime をビルドしないため)。
- `biwa build` はリンクを作らないので、Windows の開発者モードは要らない (`biwa dev` は `assets/` のリンクに要る)。

### 遊ぶ人

ネイティブ版:

- GPU のドライバ (Windows: DX12 か Vulkan、macOS: Metal、Linux: Vulkan か OpenGL ES)。
- Linux: `libfontconfig1`、X11 なら `libxkbcommon-x11-0` (デスクトップ環境なら普通は入っている)。
- macOS: 13 以降 (Info.plist の下限。ネイティブ版は WebKit に依らないので、もっと下げられる見込み)。

WebView 版:

- Windows: WebView2 Runtime (Windows 10 1803 以降 / 11 には入っている)。
- macOS: WasmGC が要るので、Safari 18.2 以上 (WKWebView は Safari と同じ WebKit を使う)。
  Ventura 13.7 なら Safari 18.6 まで更新できる (OS とは別の更新)。古い Safari のままだと動かない (§4)。
- Linux: WebKitGTK 4.1 (`libwebkit2gtk-4.1-0`)。WasmGC が動く版であること。

### runtime をビルドする (Biwa の開発者)

ネイティブ版は CI (`.github/workflows/runtime-native.yml`) が 3 OS でビルドする。手元でビルドするときは:

- 共通: Rust。`engine/desktop-rust` で `cargo build --release -p biwa-runtime`。
- Linux: `sudo apt install build-essential pkg-config libfontconfig1-dev`
- macOS: Xcode Command Line Tools。universal にするなら 2 つのターゲットでビルドして `lipo -create`。

WebView 版は CI (`.github/workflows/runtime-webview.yml`) が 3 OS でビルドする。手元でビルドするときは:

- 共通: Rust。`engine/desktop-tauri/src-tauri` で `cargo build --release`。
- Linux: `sudo apt install libwebkit2gtk-4.1-dev build-essential libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev pkg-config`
- macOS: Xcode Command Line Tools。universal にするなら 2 つのターゲットでビルドして `lipo -create`。
- Windows: Microsoft C++ Build Tools。

## 3. 使い方と検証の手順

### ネイティブ版

```sh
cd <project>
biwa build --platform windows,macos,linux
```

できた zip / tar.gz を各 OS のマシンへ持っていき、展開して起動する
(macOS の Gatekeeper・Windows の SmartScreen は WebView 版と同じく出る。下の WebView 版の説明を参照)。

確かめること:

1. 起動するか。ウィンドウのタイトルがパッケージ名か。起動しなければ `biwa-runtime-error.log` (§1)。
   - Windows はコンソールが無いので、PowerShell から `.\<name>\<name>.exe; $LASTEXITCODE` で終了コードを見る。
   - macOS は `./<name>.app/Contents/MacOS/<name>` を端末から起動すると標準エラーが見える。
2. ゲーム: タイトル → 「つぎへ」→「シーンへ」でシーンに入り、文字送りとクリック (か Enter / Space) で進むか。
   - 文字 (日本語、太字、禁則) と画像 (`assets/`) が出るか。
   - ウィンドウの大きさを変えたとき、HiDPI (Retina・Windows の拡大率) で文字がぼけないか。
3. 描画の性能 (動いている間の CPU 使用率、止まっているときに眠るか)。
4. 開いた直後からタイトルが出るか (macOS で真っ白だった件)。
5. ウィンドウの角・辺をつまんでも縦横比が保たれるか (macOS・Windows)。全画面 (F11・Alt+Enter・macOS は Cmd+Ctrl+F) で
   上下か左右に黒い帯が付き、クリックの位置がずれないか。

### WebView 版

```sh
cd <project>
biwa build --platform windows-webview,macos-webview,linux-webview
```

できた zip / tar.gz を各 OS のマシンへ持っていき、展開して次の 3 通りを試す。

| 起動                                              | 何が開くか                                                              |
| ------------------------------------------------- | ----------------------------------------------------------------------- |
| そのまま                                          | ゲーム                                                                  |
| `--probe` (または環境変数 `BIWA_PROBE=1`)         | WebView の検証ページ (ゲームの代わり)                                   |
| `--localhost` (または `BIWA_LOCALHOST=1`)         | どの OS でも `game/` を `http://localhost:<port>` で配る (macOS の既定) |
| `--custom-scheme` (または `BIWA_CUSTOM_SCHEME=1`) | どの OS でも独自スキームで配る (Windows / Linux の既定)                 |

`--localhost` / `--custom-scheme` は `--probe` と併用できる。配り方による差を切り分けるのに使う (§4)。

起動の仕方の例:

- Windows (PowerShell): `.\<name>\<name>.exe --probe`
- macOS: `open <name>.app --args --probe` (すでに起動していると引数が渡らないので、先に終了しておく)
  - ダウンロードした zip から展開したものは、初回に Gatekeeper が止める
    (ad-hoc 署名で公証していないため)。「システム設定 → プライバシーとセキュリティ」から開くことを許可する。
    同じマシンで作ったもの、USB などで運んだものは止められないことがある。
- Linux: `./<name>/<name> --probe`

検証ページは結果を表にする。「結果を JSON でコピー」で取り出せる。
開発者ツールは、配布版でも右クリックの「Inspect Element」か、
Ctrl+Shift+I (Linux / Windows)・Cmd+Option+I (macOS) で開く
(`Cargo.toml` の `devtools` feature。検証が済んだら外す)。

### 確かめること

1. 検証ページ: 「Wasm の実行」の項目がすべて OK か。とくに次の 2 つ。
   - `SharedArrayBuffer`
   - `ゲームの wasm を instantiate`
   - NG があれば `--localhost --probe` でも試す。
2. ゲーム: タイトル → 「つぎへ」→「シーンへ」でシーンに入り、文字送りとクリックで進むか。
   - 画像 (`assets/`) が出るか。
   - NG なら `--localhost` でも試す。
3. 描画の性能: 検証ページの `requestAnimationFrame` の fps と WebGL2 のレンダラ。
   - ゲームの canvas の動き (PixiJS) が滑らかか。
4. メディア (これから実装するものの下調べ): 検証ページの audio / video の `canPlayType` と `mediaCapabilities`。
5. 配布物: 別のマシンへ持っていったとき、macOS の Gatekeeper や Windows の SmartScreen でどう止まるか。
   - macOS: ad-hoc 署名が受け入れられるか (`codesign -dv --verbose=4 <name>.app` と `codesign --verify --deep <name>.app`)。

## 4. 判明したこと

### Linux (WebKitGTK 2.52, Ubuntu 24.04)

| 項目                                              | `biwa://` (既定)          | `--localhost` |
| ------------------------------------------------- | ------------------------- | ------------- |
| WasmGC・externref (最小モジュールとゲームの wasm) | OK                        | OK            |
| `crossOriginIsolated`                             | **false**                 | true          |
| `SharedArrayBuffer`                               | OK (runtime が有効にする) | OK (同)       |
| Worker + `Atomics.wait` / `notify`                | OK                        | OK            |
| WebGL2                                            | OK                        | OK            |
| ゲーム (タイトル → シーン、画像)                  | OK                        | OK            |

(Xvfb の上での確認。GPU は無く、WebGL はソフトウェア描画。
`biwa build` で組み立てた tar.gz を別の場所へ展開して起動した)

WebKit 系の問題が 3 つあり、どれも対処済み:

1. **WebKitGTK は既定で `SharedArrayBuffer` を出さない** (HTTP で COOP / COEP を付け、`crossOriginIsolated` が true でも)。
   JavaScriptCore の実行時オプション `JSC_useSharedArrayBuffer=1` で出る。
   runtime が起動時に環境変数として置く (`engine/desktop-tauri/src-tauri/src/main.rs`。WebView のプロセスが引き継ぐ)。
2. **WebKitGTK では、独自スキームに COOP / COEP を付けても `crossOriginIsolated` が false のまま**
   (`http://localhost` なら true)。
   それでも 1. の `SharedArrayBuffer` は作れて Worker にも渡せるので、
   エンジンの起動時の検査を `crossOriginIsolated` から `SharedArrayBuffer` の有無に変えた
   (`engine/nodejs/src/engine/vm/wasm/host.ts`)。
   ブラウザは isolated でないページから `SharedArrayBuffer` を隠すので、ブラウザでは同じ判定になる。
3. **JavaScriptCore の `WebAssembly.Module.imports()` は、GC の型が import に現れると例外を投げる**
   (「unable to produce import descriptors for the given module」)。compile と instantiate はできる。
   Safari / macOS の WKWebView も JavaScriptCore なので、同じことが起きるはず。
   エンジンは一覧が取れなければ、実装しているすべての import を渡す
   (`engine/nodejs/src/engine/vm/wasm/worker.ts` の `declaredImports` / `allImports`)。
   その場合、足りない syscall の名前は出ず、`LinkError` になる。

### macOS の ad-hoc 署名 (Linux の上で確認)

仮の arm64 の Mach-O を runtime として組み立て、rcodesign 0.29 の `print-signature-info` で確かめた:
フラグは `ADHOC`、識別子は Info.plist の `CFBundleIdentifier` (`dev.biwa.game.<name>`)、
Info.plist (slot 1)・要件 (slot 2)・リソース (slot 3) のダイジェストが入っている。
rcodesign の `verify` は ad-hoc の空の CMS を問題として報告するが、rcodesign 自身が
「verify は誤った結果を出す。Apple のツールを使え」と注記している。実機の `codesign` で確かめる。

### Windows (WebView2 / Edge 155, 実機, 2026-10-22)

検証ページの必須項目はすべて OK、ゲームも動いた (結果の JSON は `~/memo/desktop-test-logs/`)。

- `http://biwa.localhost` で `crossOriginIsolated` が true になり、`SharedArrayBuffer` も出る
  (WebView2 は独自スキームにも COOP / COEP を効かせる)。
- WasmGC・`Module.imports()` (26 個)・instantiate・Worker + Atomics・WebGL2 (D3D11) すべて OK。
- 起動時に **SmartScreen の警告** が出た。原因は exe に Authenticode 署名が無く、
  ダウンロードしたもの (Mark of the Web 付き) だから。どのエンジンでも同じで、
  消すにはコード署名 (評判が付くまでは署名しても出ることがある) が要る。

### macOS (Ventura 13.7 系, Apple Silicon (M1), 実機, 2026-10-22)

(このあと Safari を更新して確かめ直した。次の節)

**動かなかった。** 原因は 2 つで、別々である。`uname`: Darwin 22.6.0 (xnu-8796.141.3.713.2, 2025-07)。

1. **WasmGC: 入っている WebKit が古い (Safari 18.2 より前)。**
   - 最小モジュールの `validate()` が false。ゲームの wasm は
     「byte 14: 0th Type is non-Func, non-Struct, and non-Array -50」で失敗。
     `-50` は型の先頭バイト `0x4e` (= `rec`、再帰型のグループ。`0x4e - 128 = -50`) で、
     生成物の型セクションは `(rec ...)` で始まる。つまりこの JavaScriptCore は WasmGC が有効でない。
   - WKWebView は OS の WebKit を使い、WebKit は Safari の更新で入れ替わる。
     WasmGC が既定で有効になったのは Safari 18.2。Ventura にも Safari 18.x (最後は 18.6) が
     OS とは別の更新として来るが、この Mac には入っていないと見られる (要確認)。
   - 対処: Safari を 18.2 以上 (Ventura なら 18.6) にする。runtime 側では直せない。
     macOS の下限は「Ventura 13.7 以降 + Safari 18.2 以上」(Sonoma 14.7 以降・Sequoia 以降は標準で満たす見込み) になる。
2. **SharedArrayBuffer: 独自スキーム (`biwa://`) が cross-origin isolated と見なされない。**
   - `crossOriginIsolated` が false、`SharedArrayBuffer` が undefined。WebKitGTK と同じ挙動で、
     WebKit は COOP / COEP を HTTP(S) の応答にしか効かせないと見られる。
   - WebKitGTK のような環境変数の逃げ道は WKWebView には無い。
   - → Safari 更新後は `crossOriginIsolated` が true でも `SharedArrayBuffer` が出なかった。
     `--localhost` なら出た (次の節)。
3. 起動時の **Gatekeeper の警告** は、ad-hoc 署名で公証していないためで、想定どおり。
   - 消すには Developer ID での署名と公証が要る (`apple-codesign` でどの OS の上でもできる)。

### macOS (macOS 27, Safari 22625.1.29.11.28, Apple Silicon (M1), 実機, 2026-10-26)

上の Mac で Safari (と macOS) を更新して、もう一度確かめた (`~/memo/desktop-test-logs/biwa-probe-macos{2,3}.json`)。

| 項目                                              | `biwa://`                       | `--localhost` |
| ------------------------------------------------- | ------------------------------- | ------------- |
| WasmGC・externref (最小モジュールとゲームの wasm) | OK                              | OK            |
| `WebAssembly.Module.imports()`                    | 例外 (§4 の Linux の 3. と同じ) | 例外 (同)     |
| `crossOriginIsolated`                             | **true**                        | true          |
| `SharedArrayBuffer`                               | **NG**                          | OK            |
| Worker + `Atomics.wait` / `notify`                | NG (SAB 無し)                   | OK            |
| WebGL2                                            | OK                              | OK            |

- Safari の更新で WasmGC は動いた (1. の原因どおり)。
- 独自スキームでは `crossOriginIsolated` が true になるのに `SharedArrayBuffer` が出ない。
  WKWebView は独自スキーム (WKURLSchemeHandler) のページに `SharedArrayBuffer` を出さないと見られる。
  HTTP (`http://localhost`) で配れば出る。
- これを受けて、**macOS では runtime が localhost で配るのを既定にした** (§1)。
  `--custom-scheme` で独自スキームに戻せる。

## 5. 残っていること

- ネイティブ版: Windows / macOS の実機での確認 (段階 6)。リンクのインライン表示・ポインタの形・スクロールは未対応
  (`engine/desktop-rust/README.md`)。

- runtime の入手: `biwa` に同梱するか、版の合うものを初回にダウンロードする (GitHub Releases など)。
- エンジンの JS をビルド済みにし、開発サーバを `biwa` に内蔵する (Node.js を不要にする。メモの段階 3)。
- ゲームごとのアイコン・ウィンドウの大きさを `biwa-package.json` などから与える。
  - Windows の exe のアイコンは `editpe` (純 Rust) で差し替えられる見込み。いまは runtime の既定のアイコン。
  - macOS は `Resources/icon.icns` を差し替えるだけ。いまは `engine/desktop-tauri/src-tauri/icons/icon.icns`。
  - アイコンの元は `engine/desktop-tauri/icon.svg` (仮)。作り直すには
    `npx tauri icon icon.svg -o src-tauri/icons` を実行し、Android / iOS / Store 用は消す。
- macOS の Developer ID での署名と公証 (`apple-codesign` で、どの OS の上でもできる)、Windows のコード署名。
- Linux の AppImage (WebKitGTK を同梱して版の問題を避ける)。
- 検証が済んだら `devtools` feature と検証ページを配布物から外す (またはオプションにする)。
