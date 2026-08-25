# Biwa Engine (Node.js based edition)

ノベルゲームエンジンの基盤実装。アーキテクチャと規約は `README.md` に書いてある。
作業前に必ずそちらを読むこと。ここには作業上の注意だけを置く。

## 注意

- このディレクトリは `biwa` コマンドに丸ごと同梱され、
  `biwa dev` 実行時にプロジェクトの `.biwa_runtime/` へ展開される。
  ファイルを増やすとそのまま同梱物が増える (`cli/src/assets.rs` の exclude を参照)。
- `src/game/` は `biwa dev` が生成物で上書きする領域。
  リポジトリにある `entry.ts` はエンジン単体で起動するためのプレースホルダ。
- std から呼ばれる API は `src/engine/api/` (syscall 層) に置く。
  ここに関数を足したら `library/std` の native 実装と対で変更する。
  import は `@biwa/engine/<name>` という論理パスで書く規約 (alias で解決される)。
- パッケージマネージャは npm。利用者の環境に Node.js しか仮定しない。
- `tsconfig.json` では未使用シンボルの検査を有効にしない。
  生成された TypeScript が `src/game/` に同居するため。
- GSAP は採用しない (PixiJS Ticker と混在させると事故が起きやすい)。
