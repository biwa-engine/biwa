# Biwa Engine (Node.js based edition)

Biwa エンジンの基盤エンジン実装 (Node.js 版)。
描画は PixiJS (WebGL) と DOM、ビルドと開発サーバは Vite。

このディレクトリは `biwa` コマンドに同梱され、`biwa dev` 実行時に
プロジェクトの `.biwa_runtime/` へ展開される。Vite のルートはそこになる。
エンジン単体でも `npm run dev` で起動でき、その場合はゲームが無い状態
(`src/game/entry.ts` のプレースホルダ) で立ち上がる。

## ゲームコードとの境界

コンパイラは Biwa のコードを **wasm または TypeScript** に変換する。
ゲーム本体はエントリポイント `__biwa_entrypoint` (= `scene main`) として現れ、
`biwa dev` が生成するスタブ `src/game/entry.ts` を経由して呼ばれる。

```
src/main.ts
  ├─ kind: "wasm"        → runWasm(url)                     ← Worker で wasm を走らせる
  └─ kind: "typescript"  → kernel.run(entrypoint(game))     ← scene main (generator)
```

どちらで来ても、エンジン API の実装 (`src/engine/api/*`) は同じものが呼ばれる。
違うのは「どこで動いていて、どうやって中断するか」だけである。

エンジンが知っているのは以下だけで、パッケージ名や生成されたシンボル名は知らない。

| 規約                              | 実体                                                                      |
| --------------------------------- | ------------------------------------------------------------------------- |
| `src/game/entry.ts`               | `biwa dev` が生成するスタブ。default export が `BiwaBackend`              |
| `@biwa/engine/<path>`             | std から参照されるエンジン実装。Vite alias で解決 (TypeScript ターゲット) |
| `biwa:engine` / `biwa:runtime`    | wasm の import 名前空間 (wasm ターゲット)                                 |
| `BiwaGame` (`src/engine/game.ts`) | `std::game::Game` に対応する構造                                          |

## エンジン API は syscall である

ゲームコードから見ると、エンジンの API はシステムコールにあたる。
呼ぶとエンジン側 (kernel land) の処理に入り、
API によっては完了までブロックし、API によっては処理を積んで直ちに返る。

中断をどう作るかはターゲットで違う。

### TypeScript ターゲット

**scene は generator function として出力される。** これが VM にあたる。
`yield` が syscall 命令で、kernel が `next()` で結果を書き戻して再開する。

```ts
// 生成された scene
export function* main(g) {
  yield write("こんにちは。"); // ← レジスタに積んで VM exit
  yield wait(); // ← クリックが来るまで再開しない
  return g;
}
```

| 層                        | 役割                                     | 対応するもの         |
| ------------------------- | ---------------------------------------- | -------------------- |
| std (`base_engine.biwa`)  | syscall の記述子を組み立てる             | レジスタに引数を積む |
| コンパイラ (codegen)      | scene の中に `yield` を置く              | syscall 命令         |
| `src/engine/vm/kernel.ts` | 記述子を処理し、結果を書き戻して再開する | kernel land          |

```
src/engine/vm/
  syscall.ts   # syscall 番号と記述子の型 (std との唯一の合意点)
  kernel.ts    # VM を回すループ
  handlers.ts  # syscall 番号 → 実装の対応表
src/engine/api/
  context.ts   # 現在のエンジン実体 (syscall の実装が参照する)
  message.ts   # writeMessage / waitForClick
  image.ts     # createImage
```

現状の syscall:

| syscall     | 対応する std の関数              | ふるまい                 |
| ----------- | -------------------------------- | ------------------------ |
| `Sys.Write` | `base_engine::write` (lang item) | 中断しない               |
| `Sys.Wait`  | `base_engine::wait` (lang item)  | クリックまで**中断する** |

実装が Promise を返せばブロッキング syscall で、解決するまで scene を再開しない。
値をそのまま返せば非ブロッキング syscall で、scene はそのまま走り続ける。
非ブロッキング syscall が続いてフレームを落とさないよう、
8ms を超えたら一度 `requestAnimationFrame` に制御を返す。

### 中断できるのは scene の中だけ

generator にしているのは scene だけなので、

- 中断する syscall は scene の中にしか現れない (novel statement の展開先なので構文上そうなる)
- 中断しない API (画像を出す、名前を変える) は普通の関数呼び出しでよい。
  std の native 実装が `@biwa/engine/api/*` を直接呼ぶ
- 普通の `fn` からブロッキング API を呼ぶことはできない

kernel が `next()` を呼ばない限り VM は止まったままなので、
ポーズ・スキップ・オート・速度調整はこのループの外側で決められる。
`scene.return()` で巻き戻せば、シーンの強制終了もできる。

### wasm ターゲット

生成物は WasmGC を使った 1 つのモジュールで、エンジン API はホスト関数の import になる。

```wat
(import "biwa:engine" "sys_write" (func $sys_write (param externref)))
(import "biwa:engine" "sys_wait"  (func $sys_wait))
```

wasm 自身には中断の仕組みが無いので、**wasm を Worker で走らせ、
ブロッキング syscall ではそのスレッドを `Atomics.wait` で止める**。
メインスレッドが処理して結果を `SharedArrayBuffer` に書き、`Atomics.notify` で起こす。
止まるのは Worker だけなので、その間も描画とイベント処理は動き続ける。

```
src/engine/vm/wasm/
  contract.ts  # import 名 → 区分 (local / cast / call)。std との合意点
  bridge.ts    # SAB のプロトコル
  worker.ts    # wasm の instantiate と実行。ここが VM の中
  host.ts      # メインスレッド側の kernel。syscall を api/* に流す
```

| import                                          | 区分         | 実行される場所                    |
| ----------------------------------------------- | ------------ | --------------------------------- |
| `biwa:runtime` `string_const`                   | -            | Worker (memory から UTF-8 を復号) |
| `biwa:engine` `sys_write`                       | 積んで返る   | Main                              |
| `biwa:engine` `sys_wait`                        | **中断する** | Main                              |
| `biwa:engine` `sys_create_image`                | 積んで返る   | Main                              |
| `biwa:engine` `sys_string_concat` / `sys_map_*` | -            | Worker                            |

Worker 側に置くものがあるのは、`externref` / `anyref` が JS のオブジェクト参照
そのもので**スレッド境界を越えられない**からである。
`String` は externref、`Map` / `Option` は externref / anyref なので、
それらを触るだけの操作は wasm と同じスレッドに置くしかない。

`SharedArrayBuffer` を使うため、ページは cross-origin isolated である必要がある
(`vite.config.ts` が COOP/COEP を送っている)。**配信するサーバでも同じヘッダが要る。**

制限:

- 初期 `Game` を渡せない。WasmGC の struct を JS から作れないので `null` を渡している。
  scene が `g` のフィールドを読むと trap する
- ブロッキング syscall が返せるのは JSON にできる値だけである

設計の検討過程 (`node:vm` が使えない理由、Worker + `Atomics.wait` や
QuickJS + ASYNCIFY との比較、セーブ・ロードの方針) は
[`docs/execution-model.md`](../../docs/execution-model.md) にある。

## アセット

アセットはゲームのパッケージ直下の `assets/` に置かれる (`src/` の兄弟)。
`.biwa` が書くパスは**その `assets/` を基準とした相対パス**である。

`biwa dev` はそのディレクトリを `.biwa_runtime/public/assets` へリンクする。
`public/` の中身はそのまま URL のルートに出るので、
エンジンから見たアセットの位置は常にこうなる。

```
.biwa が書いたパス  →  `${import.meta.env.BASE_URL}assets/<path>`

  "bg/room.png"     →  /assets/bg/room.png
```

この変換は `src/engine/api/assets.ts` の `resolveAssetUrl()` 1 箇所にある。
画像以外の syscall (音・動画) が増えても同じ規約に従わせるためで、
アセットを読む API は必ずここを通すこと。

`..` で `assets/` の外へ出るパスは弾く。
将来はコンパイラがパスの実在も含めて静的に検査する。

## ディレクトリ構成

```
src/
  main.ts             # 起動: レンダラ初期化 → コンテキスト登録 → ターゲットに応じて実行
  engine/
    game.ts           # ゲームコードとの境界の型 (BiwaGame / BiwaEntrypoint / BiwaBackend)
    vm/               # scene を駆動する kernel と syscall の定義
    vm/wasm/          # wasm 生成物を Worker で走らせる側
    api/              # syscall の実装 (std からも `@biwa/engine/api/*` として呼ばれる)
    api/assets.ts     # `.biwa` が書くアセットのパスを URL に直す
    Renderer.ts       # PixiJS Application のラッパー
    LayerManager.ts   # Canvas/DOM レイヤーの生成・参照管理
    CommandQueue.ts   # 使っていない。VM 化以前の名残
    tween.ts          # PixiJS Ticker ベースの線形補間
  components/
    ComponentRegistry.ts
    TextBox.ts        # メッセージウィンドウ
  commands/           # 暫定。CommandQueue 用のコマンド群
  game/
    entry.ts          # `biwa dev` が生成して上書きする (リポジトリのものはプレースホルダ)
```

## レイヤーシステム

Canvas レイヤーと DOM レイヤーを `z-index` で任意に積み重ねる。

- **Canvas レイヤー**: スプライト・エフェクト描画 (PixiJS Container)
- **DOM レイヤー**: テキスト・UI 描画 (HTMLElement, `pointer-events: none`)

現在は `main.ts` に直書きしている。
レイヤーと UI コンポーネントの定義は将来 XML で行う。

```xml
<layers>
  <layer id="background"  type="canvas" />
  <layer id="chara"       type="canvas" />
  <layer id="message"     type="dom"    />
</layers>
```

## 技術スタック

| 用途                 | ライブラリ                  |
| -------------------- | --------------------------- |
| ビルド               | Vite + TypeScript (vanilla) |
| 描画                 | PixiJS v8 (WebGL/Canvas)    |
| アニメーション       | PixiJS Ticker (自前 tween)  |
| デスクトップ出力     | Tauri (予定)                |
| パッケージマネージャ | npm                         |

- GSAP は採用しない (PixiJS Ticker と混在させると事故が起きやすいため)。
- npm を使うのは、利用者の環境に Node.js しか仮定しないため。

## 今後の実装予定

- [ ] セーブ・ロード (syscall ログの記録と再生)
- [ ] XML によるレイヤー・コンポーネント定義のローダー
- [ ] `std::game::window` の native (`MessageWindow` / `Canvas`) の実装
- [ ] `characters` / `states` の初期化をゲーム側から渡す口
- [ ] tween のイージング対応
- [ ] Tauri アダプター (ファイルアクセスの抽象化)
