# Biwa CLI

`biwa` コマンドを提供する。

Biwa でゲームを作るのに必要なもの (コンパイラ・エンジン・標準ライブラリ) は
すべてこのバイナリ (とツールチェインの runtime) が持っている。
`biwa dev` と打てばゲームが動くところまで行けることを目標とする。
開発 (`biwa dev`。ネイティブでもブラウザでも) と配布物のビルド (どの形態も) に Node.js は要らない
(要るのはエンジン自体を開発する人と CI だけ)。
デスクトップ版のビルド (`biwa build`) も Rust などのビルド環境を要らない。
OS ごとのエンジン本体 (runtime) は Biwa の CI がビルドしておき、
`biwa build` はそれにゲームのデータを添えて組み立てるだけだからである (後述)。

コンパイラ biwac は**ライブラリとして**呼ぶ (`biwac_driver::compile`)。
`biwac` コマンドを起動することはない。

## コマンド

### `biwa dev`

開発中のゲームを動かし、ソースが変わるたびに建て直す。カレントディレクトリ (または `--package-path`) から
上へ辿って `biwa-package.json` を探し、それをルートパッケージとして扱う。

動かし方は `--platform` で選ぶ。

| `--platform`    | 動かし方                                                                       | Node.js |
| --------------- | ------------------------------------------------------------------------------ | ------- |
| `native` (既定) | ツールチェインのネイティブの runtime (この OS 用) をウィンドウで動かす         | 要らない |
| `web`           | ツールチェインのビルド済みの Web runtime を内蔵の HTTP サーバで配り、ブラウザで動かす | 要らない |

#### `--platform native` (既定)

1. **runtime を探す** — ツールチェインの `native-<この OS>` の runtime (`biwa build` と同じ。`src/toolchain.rs`)。
2. **std を揃える** — 標準ライブラリを `<project>/.biwa_build/deps/std/` に展開する (下の `web` の 1. と同じ)。
3. **ビルドする** — `biwac_driver::compile`。出力は `<project>/.biwa_build/wasm/<package>.wasm`。
   最初のビルドが通らなければ始めない。
4. **runtime を起動する** — `biwa-runtime --dev --wasm <出力> --assets <project>/assets --title "<package> (dev)"`。
5. **監視する**
   - `src/` の `.biwa` が変わったら建て直し、通ったら runtime に読み直させる (標準入力に `reload` を 1 行)。
     runtime はウィンドウを開いたまま、wasm とアセットを読み直してゲームを最初からやり直す。
     書きかけのコードでビルドが失敗したときは、前のゲームを動かしたまま次の保存を待つ。
   - `assets/` の中が変わったときは、建て直さずに読み直させる。
   - ウィンドウの中で F5 を押しても読み直す。
   - ウィンドウを閉じると `biwa dev` も終わる。`biwa dev` が終わると runtime の標準入力が閉じ、runtime も終わる。

#### `--platform web`

1. **runtime を探す** — ツールチェインのビルド済みの Web runtime (`<toolchain>/runtime/web/`。`biwa build` と同じ)。
2. **std を揃えてビルドする** — `native` と同じ。最初のビルドが通らなければ始めない。
3. **HTTP サーバを立てる** (`src/serve.rs`) — 127.0.0.1 の 5173 番 (塞がっていれば次。`--port` で決め打ち) で待ち受け、
   `biwa build --platform web` の出力 (`dist/web/`) と同じ形を、ファイルを写さずにその場から配る。

   | パス                 | 中身                                                                          |
   | -------------------- | ----------------------------------------------------------------------------- |
   | `/biwa-game.json`    | ゲームの情報。リクエストのたびに作る (`buildId` は wasm の内容、`dev: true`)  |
   | `/game.wasm`         | コンパイラの出力 (`.biwa_build/wasm/<package>.wasm`)                          |
   | `/assets/...`        | パッケージの `assets/`                                                        |
   | `/__biwa_dev/events` | 建て直しの知らせ (Server-Sent Events)                                         |
   | それ以外             | Web runtime                                                                   |

   - wasm の実行に `SharedArrayBuffer` が要るので、すべての応答に COOP / COEP / CORP を付ける。
   - 音や動画のシークのために `Range` に答える。根の外を指すパス (`..` など) は拒む。
4. **監視する** — `.biwa` が変わったら建て直し、通ったら開いているページに読み直させる
   (`/__biwa_dev/events` に `reload`。Web runtime は `biwa-game.json` に `dev: true` があるときだけこれを聞く。
   `engine/nodejs/src/game/runtime-entry.ts`)。`assets/` が変わったときも読み直させる。
   書きかけのコードでビルドが失敗したときは、前のゲームのまま次の保存を待つ。

#### エンジンを開発するとき

`engine/nodejs` で `npm run watch:runtime` を走らせ (変更のたびに `dist-runtime/` を作り直す)、
ゲームのパッケージの中で `biwa dev --platform web --web-runtime <engine/nodejs>/dist-runtime` を使う。
`biwa dev` はそのディレクトリも監視し、作り直されたらページを読み直させる (書き終わるまで少し待つ)。

オプション:

| オプション                   | 意味                                                                                       |
| ---------------------------- | ------------------------------------------------------------------------------------------ |
| `-p`, `--package-path <DIR>` | 対象パッケージ (既定: カレントディレクトリ)                                                |
| `-r`, `--rebuild`            | キャッシュを無視して最初から建て直す                                                       |
| `--platform <PLATFORM>`      | 動かし方。`native` (既定) か `web`                                                         |
| `--toolchain <NAME>`         | runtime を取るツールチェイン (既定: `BIWA_TOOLCHAIN`、なければこの CLI と同じもの)          |
| `--port <PORT>`              | 開発サーバのポート (`web` のみ。既定: 5173 から空いているもの)                             |
| `--web-runtime <DIR>`        | `web` で、ツールチェインの代わりにこの Web runtime を使う (エンジンの開発用)                |

### `biwa build`

配布物をビルドする。パッケージの探し方は `biwa dev` と同じ。

```
biwa build                                  # → dist/<この OS>/ (--platform desktop と同じ)
biwa build --platform windows,macos,linux   # → dist/windows/ dist/macos/ dist/linux/ (ネイティブ)
biwa build --platform linux-webview         # → dist/linux-webview/ (WebView。windows-/macos- も同様)
biwa build --platform web                   # → dist/web/
```

既定がデスクトップ版なのは、配布形態の第一の目標がデスクトップアプリだからである。
`--platform` はカンマ区切りで複数並べられる。`desktop` は `biwa` が動いている OS のネイティブ版を指す。

デスクトップ版の runtime は 2 種類ある。

- **ネイティブ** (`windows` / `macos` / `linux`、既定): `engine/desktop-rust/`。
  wasm を wasmtime で動かし、wgpu で描く。ゲームのデータは wasm とアセットだけ (Node.js は要らない)。
- **WebView** (`windows-webview` / `macos-webview` / `linux-webview`): `engine/desktop-tauri/`。
  Web 版と同じもの (ビルド済みの Web runtime + ゲームのデータ) を OS の WebView で表示する。

コード生成のターゲットは `wasm` に固定している。

やることは順に:

1. **runtime を探す** — 時間のかかるビルドの前に、指定された形態の runtime がツールチェインに揃っているかを
   確かめる (下記)。Web 版と WebView 版はビルド済みの Web runtime (`<toolchain>/runtime/web/`) を使う。
2. **std を揃えてビルドする** — `biwa dev` と同じ。出力は `.biwa_build/wasm/<package>.wasm`。
3. **ゲームのデータを作る** — どれも `game.wasm` (2. の出力の写し)・`assets/` のコピー・`biwa-game.json`
   (名前・タイトル・版・`buildId` (wasm の内容から決まる値)・ツールチェインの名前)。
   - Web 版: ビルド済みの Web runtime を写した上に置き、それがそのまま `dist/web/` になる。Web runtime はゲームを
     焼き込まず、ページの横の `biwa-game.json` と `game.wasm` を実行時に読む (`engine/nodejs/src/game/runtime-entry.ts`)。
     `base` が `./` なので、出力はどこに置いても動く。
   - WebView 版: Web 版と同じものに検証ページを足す (`.biwa_build/game-data/webview/`)。
   - ネイティブ版: `game.wasm`・`assets/`・`biwa-game.json` だけ (`.biwa_build/game-data/native/`)。
   - 配布物は自己完結していなければならないので、アセットはリンクではなくコピーにする。
4. **(デスクトップ版のみ) 組み立てる** — 下記。

Vite (Node.js) は使わない。Web runtime は Biwa の CI (`.github/workflows/runtime-web.yml`) がエンジンの JS を
1 度だけビルドしたもので、ゲームを作る人の手元ではビルドしない。

`--platform web` の出力 (`dist/web/`) は静的ファイルなので、そのまま HTTP で配信できる。
ただし wasm の実行に `SharedArrayBuffer` が要るので、配信側で
`Cross-Origin-Opener-Policy: same-origin` と
`Cross-Origin-Embedder-Policy: require-corp` を付けること (`file://` で開いても動かない)。

#### デスクトップ版

エンジン本体は、ゲームに依らない汎用の実行ファイル「runtime」である。
runtime は実行ファイルの横の `game/` を読んで動くので、ゲームごとにビルドし直す必要が無い。
Biwa の CI (`.github/workflows/runtime-native.yml` / `runtime-webview.yml`、配るときは `release.yml`) が OS ごとに 1 度だけビルドし、
`biwa build` はそれにゲームのデータを添えて組み立てるだけである。
**コンパイルが走らないので、どの OS の上でも全 OS 向けの配布物を作れる。**

1. ゲームのデータ (`game/` になるもの) を作る (上の 3.)。
   - ネイティブ: `game.wasm`・`assets/`・`biwa-game.json` (`.biwa_build/game-data/native/`)。
   - WebView: Web 版と同じものに、WebView の検証ページ (`__biwa_probe/`、ゲームの wasm の写し付き)
     を足す (`.biwa_build/game-data/webview/`)。
2. OS ごとに組み立てて固める。WebView 版は `dist/<os>-webview/` に `<name>-<os>-webview-<arch>.*` でできる。

| OS      | `dist/<os>/` にできるもの                                                            |
| ------- | ------------------------------------------------------------------------------------ |
| Windows | `<name>/<name>.exe` + `<name>/game/`、それを固めた `<name>-windows-x86_64.zip`       |
| macOS   | `<name>.app` (`Contents/Resources/game/`)、それを固めた `<name>-macos-universal.zip` |
| Linux   | `<name>/<name>` + `<name>/game/`、それを固めた `<name>-linux-x86_64.tar.gz`          |

- macOS の .app はデータを足すと署名が合わなくなるので、組み立てたあとで
  [`apple-codesign`](https://crates.io/crates/apple-codesign) (純 Rust) で ad-hoc 署名し直す。
  どの OS の上でもできる。Apple Silicon で起動するのに最低限要る。
- zip / tar.gz に固めるのは、実行ビットと .app の構造を確実に残すためである。

runtime は CLI と同じ「ツールチェイン」のものを使う (`src/toolchain.rs`)。ツールチェインは CLI + 全ターゲットの
runtime のまとまりで、名前はビルドしたコミットの先頭 12 文字 (`biwa --version` に出る。`cli/build.rs` が焼き込む)。
コンパイラが出す wasm と runtime の間には約束 (syscall の番号など) があるので、組み合わせを揃えるためである。

```
<home>/                                   ← 環境変数 BIWA_HOME、無ければ ~/.biwa
  bin/biwa[.exe]                          ← この CLI
  toolchains/<toolchain>/runtime/<runtime-id>/biwa-runtime[.exe]
```

`<runtime-id>` は `native-{windows-x86_64,macos-universal,linux-x86_64}` と
`webview-{windows-x86_64,macos-universal,linux-x86_64}`。
Actions の `release.yml` が、このまま `~/.biwa` に展開できるアーカイブを作る (入手のしかたは `docs/desktop-build.md`)。
別のツールチェインの runtime を使うときは `--toolchain <NAME>` (または環境変数 `BIWA_TOOLCHAIN`。実験用)。

アプリの起動オプション (WebView 版の `--probe` / `--localhost`)、遊ぶ側に要るもの、
各 OS での検証状況は [`docs/desktop-build.md`](../docs/desktop-build.md) にある。

オプション:

| オプション                   | 意味                                                                                   |
| ---------------------------- | -------------------------------------------------------------------------------------- |
| `-p`, `--package-path <DIR>` | 対象パッケージ (既定: カレントディレクトリ)                                            |
| `-r`, `--rebuild`            | キャッシュを無視して最初から建て直す                                                   |
| `--platform <LIST>`          | 配布形態 (カンマ区切り)。`desktop` (既定。この OS)・`windows`・`macos`・`linux`・`windows-webview`・`macos-webview`・`linux-webview`・`web` |
| `--toolchain <NAME>`         | runtime を取るツールチェイン (既定: `BIWA_TOOLCHAIN`、なければこの CLI と同じもの。実験用) |

### `biwa publish`

パッケージを Biwa Package Hub に公開する。対話的に確認を挟みながら進む。

```
biwa publish --commit <commit-hash>
```

やることは順に:

1. **`biwa-package.json` を読む** — 名前・バージョン・説明・依存 (`min..max`)。
   バージョンは常にこのファイルの値が使われる (`--version` のような指定手段は無い)。
   同じバージョンを 2 度公開することはできない (公開は取り消せないため)。
2. **コミットを決める** — `--commit` を省略すると `HEAD` を使ってよいか確認される。
   指定・確認どちらの場合も、実在するコミットかを検証しつつ完全なハッシュに正規化する。
3. **直接依存を解決する** — `biwa-package.json` の依存はバージョン範囲でしか
   書かれていないが、hub には再現性のため正確な 1 バージョンを送る必要がある。
   「実際にこのパッケージがビルド時に使ったバージョン」を正としたいので、
   `.biwa_build/deps/<name>/` に取得済みの実体を読んで正確なバージョンと
   hub 上の id を引く (未取得なら先に `biwa dev` 等でビルドしておく必要がある)。
   依存先がまだ hub に登録されていない場合もエラーになる。
4. **(初回のみ) 公開先リポジトリを選ばせ、hub にパッケージを登録する** —
   `--update` が無ければローカルの `git remote` を見て、登録されている remote の
   URL を候補として提示する (**それ以外の URL を指定する手段は無い** —
   なりすましの入口を作らないため)。`git@host:owner/repo.git` のような SSH 形式は
   `https://host/owner/repo.git` に自動変換してから候補に出す。GitHub 等に SSH 鍵を
   通していない開発者機のほうが多いと想定されるため、hub には基本 https の URL を渡す
   (hub 側も鍵を扱わずに済む)。選んだ URL で `POST /v1/packages/` を呼ぶ。
   既に同名パッケージがあれば失敗するので、その場合は `--update` を付けて再実行する。
   `--update` があるときはこの手順自体が無い (下記)。
5. **バージョンを公開する** — `POST /v1/packages/<name>/versions/`。

オプション:

| オプション                   | 意味                                                  |
| ---------------------------- | ----------------------------------------------------- |
| `-p`, `--package-path <DIR>` | 対象パッケージ (既定: カレントディレクトリ)           |
| `--update`                   | 新規登録ではなく、既存パッケージへのバージョン追加    |
| `--commit <HASH>`            | 公開するコミット (省略時は `HEAD` を使ってよいか確認) |

`--update` を付けると、git remote の選択も新規登録もスキップし、
「hub に既に登録されているか」だけを確認してバージョンの公開に進む。
repository はパッケージに 1 つしか持てず、hub 側にもそれを変更する API が無いので、
`--update` の時点で改めて remote を選ばせる意味が無いためである
(将来バージョンごとに repository を変えられるようにする方針転換が入ったら、
そのとき初めて `--update` でも remote を選べるようにすればよい)。

ユーザ認証 (`biwa login`) はまだ無いので、今はこのコマンドを叩ける人なら誰でも
公開できる。将来 hub にアカウント機能が入ったら、`biwa publish` の前に
`biwa login` が必要になる想定である。

ハブの URL (`BIWA_HUB_URL`) はこの `biwa` バイナリのビルド時に焼き込まれる。
未設定のビルドでは `biwa publish` はエラーで止まる
(詳細は [`tools/hub/README.md`](../tools/hub/README.md))。

設定は 2 通り:

- シェルで export してビルドする: `BIWA_HUB_URL=https://hub.example.com cargo build`
- `tools/hub/client/.env` に置く (`tools/hub/client/sample.env` をコピーして編集):
  `biwa_hub_client` は path 依存として組み込まれるので、`cli`/`compiler` など
  どの workspace からビルドしても、そのビルドスクリプトの cwd は常に
  `tools/hub/client/` になる。**`.env` を置けるのはここだけ**であり、
  `cli/.env` などに置いても読まれない。`.env` は `.gitignore` 済み。
  シェルで export 済みの場合はそちらが優先され、`.env` の値は無視される。

### ターゲット

コード生成のターゲットは `wasm` に固定している。生成物は `<package>.wasm` 1 つ (単相化で std ごと入る) で、
どの runtime も同じものを走らせる (ブラウザ・WebView では Worker、ネイティブでは wasmtime)。
TypeScript のターゲット (deprecated) は `biwa` からは使えない (コンパイラには残っている)。

## ディレクトリ

```
<project>/
  biwa-package.json
  src/*.biwa                       ← ゲームのソース (監視対象)
  assets/                          ← 画像・音などのアセット
  .biwa_build/
    deps/std/                      ← CLI が用意する依存パッケージ
    wasm/<pkg>.wasm                ← biwac の出力。runtime が game.wasm として読む
    game-data/{native,webview}/    ← デスクトップ版の game/ の元 (biwa build)
  dist/
    windows/                       ← biwa build --platform windows の出力
      <name>/{<name>.exe, game/}
      <name>-windows-x86_64.zip
    macos/                         ← --platform macos
      <name>.app, <name>-macos-universal.zip
    linux/                         ← --platform linux
      <name>/{<name>, game/}, <name>-linux-x86_64.tar.gz
    windows-webview/ macos-webview/ linux-webview/   ← --platform *-webview
    web/                           ← --platform web
      index.html, _biwa/ (Web runtime), game.wasm, assets/, biwa-game.json
```

`.biwa_build/`・`dist/` はどれも生成物なので、消してよい。
以前の `biwa dev` が作った `.biwa_runtime/` (Vite のエンジンの展開先) はもう使わないので、消してよい。

## アセット

アセットはパッケージ直下の `assets/` に置く (`src/` の兄弟)。
`.biwa` から参照するときのパスは**この `assets/` を基準とした相対パス**である。

```
assets/bg/room.png   に置いたものは   create_object("bg/room.png", 0, ...)
```

runtime から見たアセットの位置は、どの形態でもゲームのデータの `assets/<path>` である。
`biwa build` は出力の `assets/` へコピーし、`biwa dev` は元の `assets/` をそのまま読ませる
(ネイティブは `--assets`、Web は内蔵のサーバの `/assets/`)。
Web runtime での解決は `engine/nodejs/src/engine/api/assets.ts` の 1 箇所にある。

`biwa dev` は `assets/` も監視し、変わったら (建て直さずに) runtime に読み直させる。

## runtime との規約

コンパイラが吐くシンボルはマングルされていてパッケージごとに名前が変わるが、
エントリポイントだけは `__biwa_entrypoint` という固定名で export される。
runtime はそれと syscall の import だけを知っていて、パッケージ名は知らない。

ゲームのデータは、どの runtime でも同じ名前で置く。

| ファイル          | 中身                                                                                 |
| ----------------- | ------------------------------------------------------------------------------------ |
| `game.wasm`       | コンパイラの出力                                                                     |
| `assets/`         | パッケージの `assets/`                                                               |
| `biwa-game.json`  | `name`・`title`・`version`・`buildId` (wasm の内容から決まる値)・`toolchain` (・`dev`) |

Web runtime は `biwa-game.json` を読んでから `game.wasm?v=<buildId>` を読む (ブラウザのキャッシュ避け)。
`dev: true` のとき (`biwa dev`) は建て直しの知らせを聞いてページを読み直す。

実行モデル (scene がどう中断するか) は
[`docs/execution-model.md`](../docs/execution-model.md) にある。

## 同梱物の扱い

std と WebView の検証ページ (`engine/desktop-tauri/probe/`) は
[rust-embed](https://docs.rs/rust-embed) でバイナリに同梱する (`src/assets.rs`)。
macOS の .app に入れるアイコンも `include_bytes!` で焼き込む (`src/desktop.rs`)。

runtime (ネイティブ・WebView・Web) は同梱しない。CI でビルドしたものを `~/.biwa/toolchains/` から使う
(`src/toolchain.rs`)。

デバッグビルドでは実行時にリポジトリ内のファイルを直接読むため、
`library/std/`・検証ページを編集すればビルドし直さずに反映される。
リリースビルドではバイナリに焼き込まれる。

## 今回の実装対象外

- `biwa new <package>`
  `<package>/biwa-package.json` と `<package>/src/main.biwa` を作る。
- `biwa build` の残り
  runtime の配布 (GitHub Releases・インストールスクリプト・`biwa` 自身による更新)、
  ゲームごとのアイコン・ウィンドウの設定、
  macOS の Developer ID での署名・公証、Windows のコード署名、
  モバイル向けの出力 ([`docs/desktop-build.md`](../docs/desktop-build.md) §5)。
- アセットの検査と選別
  将来はコンパイラがパッケージ内のパス文字列を巡回して `assets/` 以下の実在を検査し、
  必要なものの一覧を `.biwa_build/` に `.biwaassets` として吐く。
  CLI はそれを読み、`biwa dev` なら要るものだけをリンクし、
  `biwa build` なら要るものだけをコピーすることになる。
  今は検査をせず、`assets/` を丸ごとリンク (`biwa build` ではコピー) しているだけである。
