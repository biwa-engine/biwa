# Biwa CLI

`biwa` コマンドを提供する。

Biwa でゲームを作るのに必要なもの (コンパイラ・エンジン・標準ライブラリ) は
すべてこのバイナリが持っている。利用者に用意させるのは Node.js だけで、
`biwa dev` と打てばブラウザで動くところまで行けることを目標とする。

コンパイラ biwac は**ライブラリとして**呼ぶ (`biwac_driver::compile`)。
`biwac` コマンドを起動することはない。

## コマンド

### `biwa dev`

開発サーバを起動する。カレントディレクトリ (または `--package-path`) から
上へ辿って `biwa-package.json` を探し、それをルートパッケージとして扱う。

やることは順に:

1. **同梱物を揃える**
   - 標準ライブラリを `<project>/.biwa_build/deps/std/` に展開する。
     biwac は依存が `deps/` に取得済みであることを前提にビルドするが、
     取得の実装はまだ無いので、ここは CLI が肩代わりしている。
     既に `deps/std` がある場合、CLI が置いたもの (`.biwa-std-stamp` を持つもの)
     でなければ触らない。リポジトリのシンボリックリンクなどを壊さないため。
   - エンジンを `<project>/.biwa_runtime/` に展開し、初回だけ `npm install` する。
     同梱物の内容が変わったときだけ展開し直す (`.biwa-engine-stamp` で判定)。
2. **ビルドする** — `biwac_driver::compile` を呼ぶ。
   出力は `<project>/.biwa_build/<target>/` に生える
   (`wasm/<package>.wasm` または `typescript/<package>.ts`)。
3. **生成物をエンジンに渡す** — `.biwa_runtime/src/game/` に配り、
   エントリポイントのスタブ `entry.ts` を生成する (後述)。
4. **Vite を起動する** — `.biwa_runtime/` をルートとして開発サーバが立つ。
5. **監視する** — `<project>/src/` 以下の `.biwa` が変わったら 2〜3 を繰り返す。
   Vite が差分を拾ってブラウザをリロードする。
   書きかけのコードでビルドが失敗しても開発サーバは動き続ける。

オプション:

| オプション                   | 意味                                        |
| ---------------------------- | ------------------------------------------- |
| `-p`, `--package-path <DIR>` | 対象パッケージ (既定: カレントディレクトリ) |
| `-r`, `--rebuild`            | キャッシュを無視して最初から建て直す        |
| `--port <PORT>`              | 開発サーバのポート                          |
| `--target <TARGET>`          | コード生成のターゲット (既定: `wasm`)       |

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

| オプション                   | 意味                                                |
| ----------------------------- | --------------------------------------------------- |
| `-p`, `--package-path <DIR>`  | 対象パッケージ (既定: カレントディレクトリ)          |
| `--update`                    | 新規登録ではなく、既存パッケージへのバージョン追加   |
| `--commit <HASH>`             | 公開するコミット (省略時は `HEAD` を使ってよいか確認)|

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

| ターゲット    | 生成物                                        | 実行のされ方                                      |
| ------------- | --------------------------------------------- | ------------------------------------------------- |
| `wasm` (既定) | `<package>.wasm` 1 つ (単相化で std ごと入る) | Worker で走る。エンジン API はホスト関数の import |
| `typescript`  | `<package>.ts` と依存の `.ts`                 | メインスレッドで走る。scene は generator          |

どちらも見た目の挙動は同じで、syscall の実装もエンジン側で共有している。
違うのは中断の作り方である
([`docs/execution-model.md`](../docs/execution-model.md))。

中間生成物もターゲットごとに分かれているので、切り替えても互いのキャッシュは壊れない。

## ディレクトリ

```
<project>/
  biwa-package.json
  src/*.biwa                       ← ゲームのソース (監視対象)
  assets/                          ← 画像・音などのアセット
  .biwa_build/
    deps/std/                      ← CLI が用意する依存パッケージ
    wasm/<pkg>.wasm                ← biwac の出力 (--target wasm)
    typescript/{<pkg>.ts, std.ts}  ← biwac の出力 (--target typescript)
  .biwa_runtime/                   ← エンジン (Vite プロジェクト) の展開先
    public/assets → ../../assets   ← 上の assets/ へのシンボリックリンク
    src/engine/{vm,api}/*.ts       ← kernel と syscall の実装
    src/game/{game.wasm, entry.ts} ← 配られた生成物 (typescript なら *.ts)
```

`src/game/` には**今のターゲットの生成物だけ**が残る。
ターゲットを切り替えたときに古いほうが型検査に混ざると、
どちらが動いているのか分からなくなるためである。

`.biwa_build/` も `.biwa_runtime/` も生成物なので、消してよい。

## アセット

アセットはパッケージ直下の `assets/` に置く (`src/` の兄弟)。
`.biwa` から参照するときのパスは**この `assets/` を基準とした相対パス**である。

```
assets/bg/room.png   に置いたものは   create_object("bg/room.png", 0, ...)
```

`biwa dev` は起動時に `assets/` を
`.biwa_runtime/public/assets` へシンボリックリンクする (無ければ作る)。
エンジンは Vite プロジェクトなので `public/` の中身がそのまま URL のルートに出る。
結果としてエンジンから見たアセットの位置は常に `<base>assets/<path>` になり、
`.biwa` が書いたパスに `assets/` を被せるだけで引ける
(解決はエンジン側の `src/engine/api/assets.ts` 1 箇所)。

コピーではなくリンクにしているので、`assets/` にファイルを足しても
`biwa dev` を建て直す必要はない。監視の対象にもしていない。

## エンジンとの規約

コンパイラが吐くシンボルはマングルされていてパッケージごとに名前が変わるが、
エントリポイントだけは `__biwa_entrypoint` という固定名で export される
(TypeScript の export でも wasm の export でも同じ名前である)。

CLI はこれをさらに固定の形へ均したスタブ `.biwa_runtime/src/game/entry.ts` を生成する。
エンジンが見るのはこのファイルの default export だけで、
パッケージ名もターゲットもここから読み取る。

```ts
// --target wasm
import type { BiwaBackend } from "../engine/game";
import wasmUrl from "./game.wasm?url";

const backend: BiwaBackend = {
  kind: "wasm",
  packageName: "<package>",
  url: wasmUrl,
  buildId: "<生成物の内容から決まる値>",
};

export default backend;
```

```ts
// --target typescript
import { __biwa_entrypoint } from "./<package>.ts";
import type { BiwaBackend, BiwaEntrypoint } from "../engine/game";

const backend: BiwaBackend = {
  kind: "typescript",
  packageName: "<package>",
  entrypoint: __biwa_entrypoint as unknown as BiwaEntrypoint,
};

export default backend;
```

`buildId` は wasm の中身から決まる値である。ブラウザのキャッシュ避けのほかに、
**このファイルが変わることで Vite がページを作り直す**という役目がある。
`.wasm` は Vite のモジュールグラフでは葉なので、これが無いと
再ビルドしても画面が古いままになる。

実行モデル (scene がどう中断するか) は
[`docs/execution-model.md`](../docs/execution-model.md) にある。

std の native TypeScript がエンジンを参照するときは
`@biwa/engine/<path>` という論理パスで書く。
これは `.biwa_runtime/vite.config.ts` の alias と `tsconfig.json` の paths で
`.biwa_runtime/src/engine/<path>.ts` に解決される。
生成物の物理的な配置に import が依存しないようにするための仕組みである。

## 同梱物の扱い

エンジンと std は [rust-embed](https://docs.rs/rust-embed) でバイナリに同梱する。

デバッグビルドでは実行時にリポジトリ内のファイルを直接読むため、
`engine/nodejs/` や `library/std/` を編集すればビルドし直さずに反映される。
リリースビルドではバイナリに焼き込まれる。

## 今回の実装対象外

- `biwa new <package>`
  `<package>/biwa-package.json` と `<package>/src/main.biwa` を作る。
- `biwa build`
  配布用にゲームを 1 つのアプリケーションとして出力する。
- アセットの検査と選別
  将来はコンパイラがパッケージ内のパス文字列を巡回して `assets/` 以下の実在を検査し、
  必要なものの一覧を `.biwa_build/` に `.biwaassets` として吐く。
  CLI はそれを読み、`biwa dev` なら要るものだけをリンクし、
  `biwa build` ならコピーすることになる。
  今は検査をせず、`assets/` を丸ごとリンクしているだけである。
