# デスクトップ版のビルド (ビルド済みプレイヤー方式) と WebView の検証

ゲームを Windows / macOS / Linux のデスクトップアプリとしてビルドする仕組みと手順、
各 OS の WebView で Biwa が動くかを確かめる手順のメモ。
方針は `docs/biwa-distribution-targets.md` (なぜ WebView か) と
`docs/biwa-prebuilt-player.md` (なぜビルド済みプレイヤーか) にある。

状況:

- **Linux (Ubuntu 24.04, WebKitGTK 2.52)**: プレイヤーのビルド、配布物の組み立て、起動、タイトルからシーンまでの進行を確認済み。
- **Windows / macOS**: この Linux の上で配布物を組み立てられることまで確認済み
  (プレイヤーの代わりに仮の実行ファイルを使った。macOS は ad-hoc 署名の形式も確認)。
  本物のプレイヤー (CI でビルド) での組み立てと、実機での起動は未検証。

## 1. 構成

```
biwa build                              → dist/<host OS>/   (既定。--platform desktop と同じ)
biwa build --platform windows,macos,linux → dist/windows/ dist/macos/ dist/linux/
biwa build --platform web               → dist/web/
```

### プレイヤー (`engine/desktop-tauri/`)

- ゲームを焼き込まない汎用の実行ファイル (Tauri v2)。**Biwa の CI が OS ごとに 1 度だけビルドする**
  (`.github/workflows/player.yml`)。利用者の手元で Rust のビルドは走らない。
  - デスクトップ向けの実装は今後増えうるので、ディレクトリ名は実装 (Tauri) に結びつけている。
- 実行ファイルの横の `game/` (macOS は `<name>.app/Contents/Resources/game/`) を、
  独自スキーム `biwa://localhost/` (Windows は `http://biwa.localhost/`) で配る。
  `--game-dir <DIR>` (または `BIWA_GAME_DIR`) で場所を上書きできる。
  - 配るときに COOP / COEP / CORP のヘッダを付ける (`src-tauri/src/serve.rs`)。
  - 音や動画のシークのために `Range` に答える。
  - `game/` の外のパスは拒む。
- `game/biwa-game.json` (`biwa build` が書く) からウィンドウのタイトルなどを読む。
- `game/` が無ければ、案内のページ (`placeholder/`) を出す。
- 実行ファイルの大きさ: Linux で約 3.6 MB。

### `biwa build` (`cli/src/build.rs`, `desktop.rs`, `player.rs`)

1. コンパイルし、Web の静的ビルド (`vite build`) を作る。
   - `base` は `./`。Vite が出すもの (JS・wasm・Worker) は `_biwa/` に置く
     (`engine/nodejs/vite.config.ts` の `build.assetsDir`。既定の `assets/` はパッケージの `assets/` と重なるため)。
   - パッケージの `assets/` は出力へコピーする (開発時のようなリンクにはしない)。
2. デスクトップ版では、それに `biwa-game.json` と検証ページ (`__biwa_probe/`、ゲームの wasm の写し付き) を足して
   ゲームのデータにする (`.biwa_runtime/dist-game/`)。
3. OS ごとに、ビルド済みのプレイヤーにゲームのデータを添えて組み立て、固める。

| OS      | `dist/<os>/` にできるもの                                                      |
| ------- | ------------------------------------------------------------------------------ |
| Windows | `<name>/<name>.exe` + `<name>/game/`、それを固めた `<name>-windows-x86_64.zip` |
| macOS   | `<name>.app` (ad-hoc 署名済み)、それを固めた `<name>-macos-universal.zip`      |
| Linux   | `<name>/<name>` + `<name>/game/`、それを固めた `<name>-linux-x86_64.tar.gz`    |

- **組み立てにコンパイルは要らないので、どの OS の上でも全 OS 向けに作れる。**
- macOS の .app はデータを足すと署名が合わなくなるので、組み立てたあとで `apple-codesign` (純 Rust) で
  ad-hoc 署名し直す (Apple Silicon で起動するのに最低限要る)。
  Info.plist・リソース (`Resources/game/` を含む) のダイジェストが署名に入る。
- 固めるのは、実行ビットと .app の構造を確実に残すため (CI の成果物やコピーの途中で実行ビットは落ちやすい)。

### プレイヤーの置き場所

`biwa build` は次の順に探す。

1. `--players-dir <DIR>`
2. 環境変数 `BIWA_PLAYERS_DIR`
3. `~/.biwa/players`

```
<players>/windows-x86_64/biwa-player.exe
<players>/macos-universal/biwa-player
<players>/linux-x86_64/biwa-player
```

CI の成果物 `biwa-player-<id>` を取り出して、`<players>/<id>/` に置く。
将来は `biwa` に同梱するか、版の合うものを初回にダウンロードする。

## 2. 準備

### ゲームをビルドする人

- `biwa` (リポジトリの `cli/` で `cargo build --release`。いまは手元でビルドする)。
- Node.js (npm 付き)。静的ビルドに使う (いずれ不要にする。メモの段階 3)。
- ビルド済みのプレイヤー (上記)。
- **Rust も C++ Build Tools も Xcode も要らない** (プレイヤーをビルドしないため)。
- `biwa build` はリンクを作らないので、Windows の開発者モードは要らない (`biwa dev` は `assets/` のリンクに要る)。

### 遊ぶ人

- Windows: WebView2 Runtime (Windows 10 1803 以降 / 11 には入っている)。
- macOS: WasmGC が要るので、WebKit が Safari 18.2 相当以上 (Info.plist の下限は仮に 13.0。要確認)。
- Linux: WebKitGTK 4.1 (`libwebkit2gtk-4.1-0`)。WasmGC が動く版であること。

### プレイヤーをビルドする (Biwa の開発者)

CI (`.github/workflows/player.yml`) が 3 OS でビルドする。手元でビルドするときは:

- 共通: Rust。`engine/desktop-tauri/src-tauri` で `cargo build --release`。
- Linux: `sudo apt install libwebkit2gtk-4.1-dev build-essential libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev pkg-config`
- macOS: Xcode Command Line Tools。universal にするなら 2 つのターゲットでビルドして `lipo -create`。
- Windows: Microsoft C++ Build Tools。

## 3. 使い方と検証の手順

```sh
cd <project>
biwa build --platform windows,macos,linux
```

できた zip / tar.gz を各 OS のマシンへ持っていき、展開して次の 3 通りを試す。

| 起動                                      | 何が開くか                                                                |
| ----------------------------------------- | ------------------------------------------------------------------------- |
| そのまま                                  | ゲーム                                                                    |
| `--probe` (または環境変数 `BIWA_PROBE=1`) | WebView の検証ページ (ゲームの代わり)                                     |
| `--localhost` (または `BIWA_LOCALHOST=1`) | `game/` を `http://localhost:<空きポート>` で配る。`--probe` と併用できる |

`--localhost` は、独自スキーム (`biwa://`) で `SharedArrayBuffer` が出ない WebView のための逃げ道である (§4)。

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

| 項目                                              | `biwa://` (既定)            | `--localhost` |
| ------------------------------------------------- | --------------------------- | ------------- |
| WasmGC・externref (最小モジュールとゲームの wasm) | OK                          | OK            |
| `crossOriginIsolated`                             | **false**                   | true          |
| `SharedArrayBuffer`                               | OK (プレイヤーが有効にする) | OK (同)       |
| Worker + `Atomics.wait` / `notify`                | OK                          | OK            |
| WebGL2                                            | OK                          | OK            |
| ゲーム (タイトル → シーン、画像)                  | OK                          | OK            |

(Xvfb の上での確認。GPU は無く、WebGL はソフトウェア描画。
`biwa build` で組み立てた tar.gz を別の場所へ展開して起動した)

WebKit 系の問題が 3 つあり、どれも対処済み:

1. **WebKitGTK は既定で `SharedArrayBuffer` を出さない** (HTTP で COOP / COEP を付け、`crossOriginIsolated` が true でも)。
   JavaScriptCore の実行時オプション `JSC_useSharedArrayBuffer=1` で出る。
   プレイヤーが起動時に環境変数として置く (`engine/desktop-tauri/src-tauri/src/main.rs`。WebView のプロセスが引き継ぐ)。
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

仮の arm64 の Mach-O をプレイヤーとして組み立て、rcodesign 0.29 の `print-signature-info` で確かめた:
フラグは `ADHOC`、識別子は Info.plist の `CFBundleIdentifier` (`dev.biwa.game.<name>`)、
Info.plist (slot 1)・要件 (slot 2)・リソース (slot 3) のダイジェストが入っている。
rcodesign の `verify` は ad-hoc の空の CMS を問題として報告するが、rcodesign 自身が
「verify は誤った結果を出す。Apple のツールを使え」と注記している。実機の `codesign` で確かめる。

### macOS・Windows で特に見たいこと

- macOS (WKWebView): `biwa://` で `SharedArrayBuffer` が出るか。
  WebKitGTK のような環境変数の逃げ道は WKWebView には無い。
  出なければ `--localhost` で出るか (Linux では `http://localhost` なら isolated になった)。
  `--localhost` で出るなら、それを既定にすることを考える。
- macOS: Linux の上で ad-hoc 署名した .app が Apple Silicon で起動するか。Gatekeeper の挙動。
- macOS: WasmGC が動く macOS の下限 (Safari 18.2 相当の WebKit が要るはず)。
- Windows (WebView2 = Chromium): `http://biwa.localhost` で COOP / COEP が効くか。Chromium なので問題は無い見込み。

## 5. 残っていること

- プレイヤーの入手: `biwa` に同梱するか、版の合うものを初回にダウンロードする (GitHub Releases など)。
- エンジンの JS をビルド済みにし、開発サーバを `biwa` に内蔵する (Node.js を不要にする。メモの段階 3)。
- ゲームごとのアイコン・ウィンドウの大きさを `biwa-package.json` などから与える。
  - Windows の exe のアイコンは `editpe` (純 Rust) で差し替えられる見込み。いまはプレイヤーの既定のアイコン。
  - macOS は `Resources/icon.icns` を差し替えるだけ。いまは `engine/desktop-tauri/src-tauri/icons/icon.icns`。
  - アイコンの元は `engine/desktop-tauri/icon.svg` (仮)。作り直すには
    `npx tauri icon icon.svg -o src-tauri/icons` を実行し、Android / iOS / Store 用は消す。
- macOS の Developer ID での署名と公証 (`apple-codesign` で、どの OS の上でもできる)、Windows のコード署名。
- Linux の AppImage (WebKitGTK を同梱して版の問題を避ける)。
- 検証が済んだら `devtools` feature と検証ページを配布物から外す (またはオプションにする)。
