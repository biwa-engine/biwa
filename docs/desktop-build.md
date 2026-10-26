# デスクトップ版のビルド (ビルド済みプレイヤー方式) と WebView の検証

ゲームを Windows / macOS / Linux のデスクトップアプリとしてビルドする仕組みと手順、
各 OS の WebView で Biwa が動くかを確かめる手順のメモ。
方針は `docs/biwa-distribution-targets.md` (なぜ WebView か) と
`docs/biwa-prebuilt-player.md` (なぜビルド済みプレイヤーか) にある。

状況:

- **Linux (Ubuntu 24.04, WebKitGTK 2.52)**: プレイヤーのビルド、配布物の組み立て、起動、タイトルからシーンまでの進行を確認済み。
- **CI**: 3 OS のプレイヤーのビルドに成功 (2026-10-13)。
- **Windows / macOS**: CI でビルドしたプレイヤーから、Linux の上で配布物を組み立てた
  (macOS は universal バイナリの両方のアーキテクチャに ad-hoc 署名が付くことを確認)。
  - Windows: 実機で動いた (SmartScreen の警告は出る)。§4
  - macOS: 最初は動かなかった (WebKit が古く WasmGC が無い、独自スキームで SharedArrayBuffer が出ない)。
    Safari を更新し (macOS 27 / Safari 22625)、localhost で配れば検証ページの必須項目はすべて OK になった。
    そこで macOS では localhost で配るのを既定にした (2026-10-26。このプレイヤーでの実機確認はまだ)。§4
- CI でビルドした Linux のプレイヤーから組み立てた配布物も、Linux で起動しシーンまで進むことを確認済み。

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
- 実行ファイルの横の `game/` (macOS は `<name>.app/Contents/Resources/game/`) を配る。
  `--game-dir <DIR>` (または `BIWA_GAME_DIR`) で場所を上書きできる。配り方は OS で違う。
  - Windows / Linux: 独自スキーム `biwa://localhost/` (Windows は `http://biwa.localhost/`)。
  - macOS: プレイヤーの中の HTTP サーバで `http://localhost:<port>/`。
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

### 配布物の中身 (`game/` の構成)

`game/` は Web の静的ビルド (`biwa build --platform web` の `dist/web/` と同じもの) に、
プレイヤー用の 2 つを足しただけである。

```
<name>/                       (macOS は <name>.app/Contents/)
  <name>[.exe]                ← プレイヤー (biwa-player を改名したもの。macOS は MacOS/<name>)
  game/                       (macOS は Resources/game/)
    index.html                ← プレイヤーが最初に開くページ (必須)
    _biwa/                    ← エンジンの JS・Worker・ゲームの wasm (Vite の出力。名前にハッシュが付く)
    assets/                   ← パッケージの assets/ のコピー
    favicon.svg ほか           ← エンジンの public/ の中身
    biwa-game.json            ← プレイヤーが読むゲームの情報 (無くても動く)
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

(`name` と `version` はいまプレイヤーは読まない。`width` / `height` は `biwa build` はまだ書かない)

#### 手で組み立てる

`biwa build` を通さずに同じ状態を作れる (プレイヤーの動作の切り分けに使う)。Linux の例:

```sh
mkdir -p mygame
cp ~/.biwa/players/linux-x86_64/biwa-player mygame/mygame
chmod +x mygame/mygame
biwa build --platform web               # <project>/dist/web/ ができる
cp -r <project>/dist/web mygame/game
echo '{ "title": "mygame" }' > mygame/game/biwa-game.json   # 任意
./mygame/mygame
```

- `game/index.html` が無いと、プレイヤーは「`game/` が見つからない」案内のページを出す (標準エラーにも出る)。
- `game/` を別の場所に置いたまま試すなら `--game-dir <DIR>` (または `BIWA_GAME_DIR`)。
  例: `~/.biwa/players/linux-x86_64/biwa-player --game-dir <project>/dist/web`
- 検証ページも使うなら、`<project>/.biwa_runtime/dist-game/__biwa_probe/` (デスクトップ版の `biwa build` が作る) を
  `game/__biwa_probe/` にコピーする。
- macOS は .app の構造 (`Contents/{Info.plist, MacOS/<name>, Resources/game/}`) と署名が要るので、
  手で組み立てるより `biwa build --platform macos` を使う方がよい
  (手で触ったら `codesign --force --deep -s - <name>.app` で ad-hoc 署名し直す)。

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

CI の成果物 `biwa-player-<id>` を取り出して、`<players>/<id>/` に置く。`gh` を使うなら:

```sh
for id in windows-x86_64 macos-universal linux-x86_64; do
  gh run download <run-id> -R biwa-engine/biwa -n biwa-player-$id -D ~/.biwa/players/$id
done
```

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
- macOS: WasmGC が要るので、Safari 18.2 以上 (WKWebView は Safari と同じ WebKit を使う)。
  Ventura 13.7 なら Safari 18.6 まで更新できる (OS とは別の更新)。古い Safari のままだと動かない (§4)。
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
   - 対処: Safari を 18.2 以上 (Ventura なら 18.6) にする。プレイヤー側では直せない。
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
- これを受けて、**macOS ではプレイヤーが localhost で配るのを既定にした** (§1)。
  `--custom-scheme` で独自スキームに戻せる。

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
