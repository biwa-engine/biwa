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
  .biwa_build/
    deps/std/                      ← CLI が用意する依存パッケージ
    wasm/<pkg>.wasm                ← biwac の出力 (--target wasm)
    typescript/{<pkg>.ts, std.ts}  ← biwac の出力 (--target typescript)
  .biwa_runtime/                   ← エンジン (Vite プロジェクト) の展開先
    src/engine/{vm,api}/*.ts       ← kernel と syscall の実装
    src/game/{game.wasm, entry.ts} ← 配られた生成物 (typescript なら *.ts)
```

`src/game/` には**今のターゲットの生成物だけ**が残る。
ターゲットを切り替えたときに古いほうが型検査に混ざると、
どちらが動いているのか分からなくなるためである。

`.biwa_build/` も `.biwa_runtime/` も生成物なので、消してよい。

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
- 依存パッケージの取得
  現状 `deps/` に std を置くところまでしかやっていない。
  サードパーティのパッケージは利用者が自分で `deps/` に並べる必要がある。
