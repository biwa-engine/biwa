# Biwa Engine (Node.js based edition)

Biwa エンジンの基盤エンジン実装 (Node.js 版)。
描画は PixiJS (WebGL) と DOM、ビルドと開発サーバは Vite。

このディレクトリは `biwa` コマンドに同梱され、`biwa dev` 実行時に
プロジェクトの `.biwa_runtime/` へ展開される。Vite のルートはそこになる。
エンジン単体でも `npm run dev` で起動でき、その場合はゲームが無い状態
(`src/game/entry.ts` のプレースホルダ) で立ち上がる。

## ゲームコードとの境界

コンパイラは Biwa のコードを TypeScript に変換する。
ゲーム本体はエントリポイント `__biwa_entrypoint` (= `scene main`) として現れ、
`biwa dev` が生成するスタブ `src/game/entry.ts` を経由して呼ばれる。

```
src/main.ts
  └─ entrypoint(createInitialGame(packageName))   ← scene main
       └─ std の関数呼び出し
            └─ @biwa/engine/*  ← エンジンの API (syscall 層)
```

エンジンが知っているのは以下だけで、パッケージ名や生成されたシンボル名は知らない。

| 規約                     | 実体                                                  |
| ------------------------ | ----------------------------------------------------- |
| `src/game/entry.ts`      | `biwa dev` が生成するスタブ。default export が scene main |
| `@biwa/engine/<name>`    | std から呼ばれるエンジン API。Vite alias で解決          |
| `BiwaGame` (`src/engine/game.ts`) | `std::game::Game` に対応する構造               |

## エンジン API は syscall である

std の native 実装から呼ばれる `@biwa/engine/*` の関数は、
ゲームコードから見ればシステムコールにあたる。
呼ぶとエンジン側 (kernel land) の処理に入り、
API によっては完了までブロックし、API によっては処理を積んで直ちに返る。

```
src/engine/api/
  context.ts    # 現在のエンジン実体 (syscall の実装が参照する)
  message.ts    # showMessage / waitForClick
  image.ts      # createImage
```

現状の分類:

| API              | 対応する std の関数              | ふるまい                       |
| ---------------- | -------------------------------- | ------------------------------ |
| `showMessage`    | `base_engine::write` (lang item)  | 即座に返る                     |
| `createImage`    | `base_engine::create_image`       | 読み込みを積んで即座に返る     |
| `waitForClick`   | `base_engine::wait` (lang item)   | **本来ブロックすべきだが返る** |

## 制約: ブロッキング API が実装できていない

コンパイラが吐く scene 本文は**同期関数**である。
そのため JavaScript 側でクリックを待つ手段がなく、
`waitForClick` は警告を出して即座に返る。
結果として、現状はシーンが最後まで一気に流れる。

目指す形は VM である。scene を VM 上で走らせ、
ブロッキング syscall では VM を exit させて制御をエンジンに返し、
エンジンがクリックイベントを捕捉したら VM を再開する。
変数は TS (JS) の通常のメモリ領域で扱いたいので、
実行そのものは JavaScript の上に載せたまま、中断と再開だけを外から制御する形になる。

暫定の `CommandQueue` (`src/engine/CommandQueue.ts`) は、
scene を走らせながら描画コマンドを積み、後からエンジンが `await` で再生する仕組みだが、
プレイヤーの入力より先に分岐が確定してしまうため選択肢を扱えない。
現在ゲームの実行経路では使っていない。

## ディレクトリ構成

```
src/
  main.ts             # 起動: レンダラ初期化 → コンテキスト登録 → scene main 呼び出し
  engine/
    game.ts           # ゲームコードとの境界の型 (BiwaGame / BiwaEntrypoint)
    api/              # syscall 層 (std から `@biwa/engine/*` として呼ばれる)
    Renderer.ts       # PixiJS Application のラッパー
    LayerManager.ts   # Canvas/DOM レイヤーの生成・参照管理
    CommandQueue.ts   # 暫定。現在は未使用
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

- [ ] VM 実行モデル (ブロッキング syscall / クリック待ち)
- [ ] XML によるレイヤー・コンポーネント定義のローダー
- [ ] `std::game::window` の native (`MessageWindow` / `Canvas`) の実装
- [ ] `characters` / `states` の初期化をゲーム側から渡す口
- [ ] tween のイージング対応
- [ ] Tauri アダプター (ファイルアクセスの抽象化)
