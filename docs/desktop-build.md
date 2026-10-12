# デスクトップ版のビルド (Tauri v2) と WebView の検証

ゲームを Windows / macOS / Linux のデスクトップアプリとしてビルドする手順と、
各 OS の WebView で Biwa が動くかを確かめる手順のメモ。
方針 (なぜ Tauri か、Electron との比較など) は `~/memo/biwa-distribution-targets.md` にある。

状況: **Linux (Ubuntu 24.04, WebKitGTK 2.52) で、ビルドから起動、タイトルからシーンまでの進行を確認済み**。
Windows と macOS は未検証 (ビルドもまだ)。

## 1. 構成

```
biwa build                     → <project>/dist/desktop/   アプリとインストーラ (既定。--platform desktop と同じ)
biwa build --platform web      → <project>/dist/web/       Web の静的ビルド (どこに置いても動く相対パス)
```

- 中身は配布形態によらず同じ Web の静的ビルド (`vite build`) である。
  - `base` は `./`。サブディレクトリに置いても、WebView の独自スキームでも、そのまま解決される。
  - Vite が出すもの (JS・wasm・Worker) は `_biwa/` に置く (`engine/nodejs/vite.config.ts` の `build.assetsDir`)。
    既定の `assets/` は、パッケージの `assets/` を公開する位置と重なるため。
  - パッケージの `assets/` は出力へコピーする (開発時のようなリンクにはしない)。
- デスクトップ版のシェルは `engine/desktop-tauri/` (Tauri v2 のプロジェクト)。
  - デスクトップ向けの実装は今後増えうるので、ディレクトリ名は実装 (Tauri) に結びつけている。
    当面、`--platform desktop` はこれを使う。
  - ゲームに依らない共通の殻で、`biwa` に同梱され、`<project>/.biwa_desktop_tauri/` に展開される。
  - ゲームごとの値 (名前・版・識別子 `dev.biwa.game.<name>`・静的ビルドの場所) は
    `.biwa_desktop_tauri/biwa.tauri.conf.json` に書き出し、`tauri build --config` で重ねる。
  - COOP / COEP のヘッダは `tauri.conf.json` の `app.security.headers` で付けている。
- デスクトップ版の静的ビルド (`.biwa_desktop_tauri/frontend/`) には、WebView の検証ページ
  (`__biwa_probe/`) とゲームの wasm の写しが入る (`dist/web/` には入らない)。
- Tauri はクロスコンパイルをほぼ支援しない。**各 OS のアプリは、その OS の上でビルドする。**

### 出力 (`dist/desktop/`)

| OS      | 出力                                                     | 備考                                                                                          |
| ------- | -------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| Linux   | `<name>` (実行ファイル), `.deb`, `.rpm`, `.AppImage`     | deb / rpm は約 5 MB (システムの WebKitGTK を使う)。AppImage は WebKitGTK を抱えるので約 80 MB |
| macOS   | `<name>` (実行ファイル), `<name>.app`, `.dmg`            | 署名・公証はしていない                                                                        |
| Windows | `<name>.exe` (実行ファイル), `-setup.exe` (NSIS), `.msi` | WebView2 は Windows 10/11 に入っている前提                                                    |

作るインストーラは `--bundles deb,appimage` のように絞れる (Tauri の bundle target の名前)。

## 2. 準備

### 共通

- Rust (rustup)。リポジトリの `cli/` で `cargo build --release` して `cli/target/release/biwa` を作る。
- Node.js (npm 付き)。
- 初回の `biwa build` (デスクトップ版) は、シェルの Rust 側のビルドに数分かかる (4 コアで約 3.5 分)。
  2 回目以降は約 1 分。ビルドの中間物はプロジェクトごと (`.biwa_desktop_tauri/src-tauri/target/`, 約 1 GB)。

### Linux (Debian / Ubuntu)

```sh
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev pkg-config
```

AppImage を作るときは、Tauri が linuxdeploy などを GitHub から取ってくる (ネットワークが要る)。

### macOS

- Xcode Command Line Tools (`xcode-select --install`)。
- WasmGC が要るので、WebKit が Safari 18.2 相当以上であること (要確認。§4)。

### Windows

- Microsoft C++ Build Tools (「C++ によるデスクトップ開発」)。Rust は MSVC のツールチェイン。
- WebView2 Runtime (Windows 10 1803 以降 / 11 には入っている)。
- NSIS / WiX は Tauri が初回に取ってくる。
- `biwa build` はリンクを作らないので、開発者モードは要らない (`biwa dev` は `assets/` のリンクに要る)。

## 3. 使い方と検証の手順

```sh
cd <project>
biwa build
```

できたアプリで、次の 3 通りを試す。

| 起動                                      | 何が開くか                                                                  |
| ----------------------------------------- | --------------------------------------------------------------------------- |
| そのまま                                  | ゲーム                                                                      |
| `--probe` (または環境変数 `BIWA_PROBE=1`) | WebView の検証ページ (ゲームの代わり)                                       |
| `--localhost` (または `BIWA_LOCALHOST=1`) | 静的ビルドを `http://localhost:<空きポート>` で配る。`--probe` と併用できる |

`--localhost` は、Tauri の独自スキーム (`tauri://localhost` / `http://tauri.localhost`) で
`SharedArrayBuffer` が出ない WebView のための逃げ道である (§4)。

起動の仕方の例:

- Linux: `./dist/desktop/<name> --probe`
- macOS: `open dist/desktop/<name>.app --args --probe` (すでに起動していると引数が渡らないので、先に終了しておく)
- Windows (PowerShell): `.\dist\desktop\<name>.exe --probe`

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
5. 配布物: インストーラで入れたアプリが起動するか。
   - 別のマシンへ持っていったとき、macOS の Gatekeeper や Windows の SmartScreen でどう止まるか。

## 4. 判明したこと

### Linux (WebKitGTK 2.52, Ubuntu 24.04)

| 項目                                              | `tauri://` (既定)       | `--localhost` |
| ------------------------------------------------- | ----------------------- | ------------- |
| WasmGC・externref (最小モジュールとゲームの wasm) | OK                      | OK            |
| `crossOriginIsolated`                             | **false**               | true          |
| `SharedArrayBuffer`                               | OK (シェルが有効にする) | OK (同)       |
| Worker + `Atomics.wait` / `notify`                | OK                      | OK            |
| WebGL2                                            | OK                      | OK            |
| ゲーム (タイトル → シーン、画像)                  | OK                      | OK            |

(Xvfb の上での確認。GPU は無く、WebGL はソフトウェア描画)

WebKit 系の問題が 3 つあり、どれも対処済み:

1. **WebKitGTK は既定で `SharedArrayBuffer` を出さない** (HTTP で COOP / COEP を付け、`crossOriginIsolated` が true でも)。
   JavaScriptCore の実行時オプション `JSC_useSharedArrayBuffer=1` で出る。
   シェルが起動時に環境変数として置く (`engine/desktop-tauri/src-tauri/src/main.rs`。WebView のプロセスが引き継ぐ)。
2. **WebKitGTK では、`tauri://` に COOP / COEP を付けても `crossOriginIsolated` が false のまま**
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

### macOS・Windows で特に見たいこと

- macOS (WKWebView): `tauri://` で `SharedArrayBuffer` が出るか。
  WebKitGTK のような環境変数の逃げ道は WKWebView には無い。
  出なければ `--localhost` で出るか (Linux では `http://localhost` なら isolated になった)。
  `--localhost` で出るなら、それを既定にすることを考える。
- macOS: WasmGC が動く macOS の下限 (Safari 18.2 相当の WebKit が要るはず)。
- Windows (WebView2 = Chromium): `http://tauri.localhost` で COOP / COEP が効くか。Chromium なので問題は無い見込み。

## 5. 残っていること

- ゲームごとのアイコン・ウィンドウの大きさ・タイトルを `biwa-package.json` などから与える。
  - アイコンはいま `engine/desktop-tauri/icon.svg` (仮) から作ったもの。作り直すには
    `npx tauri icon icon.svg -o src-tauri/icons` を実行し、Android / iOS / Store 用は消す。
- macOS の署名・公証、Windows のコード署名。
- 検証が済んだら `devtools` feature と検証ページを配布物から外す (またはオプションにする)。
- シェルの Rust 側のビルドをプロジェクト間で共有する (`CARGO_TARGET_DIR` をユーザーのキャッシュに置くなど)。
- CI (GitHub Actions の 3 OS の matrix と `tauri-action`) で各 OS のアプリを作る。
