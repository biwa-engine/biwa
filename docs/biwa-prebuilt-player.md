# Biwa Engine: ビルド済みプレイヤー方式 (どの OS からでも全 OS 向けにビルドする)

検討日: 2026-10-12。`docs/biwa-distribution-targets.md` の続き。

## 問題

- Tauri v2 で `biwa build` を作った (`docs/desktop-build.md`) が、次の難点がある。
  - ビルドに Rust のツールチェーンが要る。Windows では Rust が MSVC (C++ Build Tools) に依存する。
  - クロスコンパイルをほぼ支援しない。各 OS のアプリはその OS の上でしか作れない。
- Biwa の利用者は Windows が多く、macOS・Linux は少ないと見込む。
- 既存のノベルゲームエンジンは「Windows 用しか出せない」「良くて Windows と macOS」で、
  クロスプラットフォームのビルドが厳しかった。Biwa はここを改善したい。

## 理想 (B)

Biwa のバイナリ (ホスト OS ごとに別) を入れるだけで、dev モードも全 OS 向けのリリースビルドもできる。

## 結論: ゲームのビルドでエンジンをコンパイルしない

- 難しさの原因は「エンジンを何で書くか」ではなく、**利用者の手元でエンジンをコンパイルしていること**。
  Tauri が Rust を要求するのは、フロントエンドをコンパイル時にバイナリへ焼き込む作りのため。
- ゲームは「wasm + アセット」というデータである (Wasm を Tier1 にした分離の成果)。
  - `game.wasm` は単相化で std ごと 1 ファイル。import はエンジンの syscall だけ。
  - エンジンの JS がゲームに依存するのは、生成するスタブ `entry.ts` (パッケージ名・wasm の URL・`buildId`) だけ。
    実行時の設定に移せば、エンジンの JS は Biwa のリリース時にビルドしておける。
- そこで **OS ごとのエンジン本体 (プレイヤー) を Biwa の CI で各 OS の上でビルドしておき、
  `biwa build` は「プレイヤー + エンジンの JS + game.wasm + assets」を組み立てるだけにする。**
  - 前例: Godot (export template)、Ren'Py、LÖVE (fused exe)、RPG ツクール MV/MZ (NW.js)。
- この形では「利用者に Rust / Go を入れさせる」(案 A) はどの実装でも不要になる。

## 全体像

```
biwa (ホスト OS ごとのバイナリ 1 つ)
 ├ コンパイラ (biwac)
 ├ ビルド済みのエンジンの JS 一式 (Vite の出力)          ← 段階 3
 ├ 開発サーバ (HTTP + 再読み込みの通知。Node は使わない)   ← 段階 3
 └ 各 OS のプレイヤー (同梱するか、初回にダウンロード)     ← 段階 2

biwa dev    : コンパイル → 開発サーバで配信 → ホスト OS のプレイヤー (かブラウザ) で表示
biwa build  : コンパイル → 対象 OS のプレイヤー + エンジンの JS + game.wasm + assets を組み立てる
```

- `biwa dev` を配布版と同じ WebView のプレイヤーで表示できるので、
  「dev では正しいのに配布版だけ崩れる」を、その OS の上では避けられる。
- Vite (HMR) はエンジン自体の開発者向けの経路として残す。

## どの OS からでも組み立てられるか

| 対象    | 組み立て                                                                                         | 署名など                                                                                                                                                                                                                                                                                |
| ------- | ------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows | exe + データのフォルダ → zip。アイコンや版情報は `editpe` (純 Rust) で書き換えられる (Wine 不要) | Authenticode は任意 (無いと SmartScreen が警告)。osslsigncode / jsign は Linux・macOS でも動く                                                                                                                                                                                          |
| macOS   | `.app` はディレクトリと Info.plist なので組み立てられる。プレイヤーは universal で用意           | **データを足すと署名が壊れるので、組み立て後に署名し直す。** `apple-codesign` (rcodesign、純 Rust) で Linux・Windows から署名・公証ができる (Godot も採用)。Apple Silicon では最低でも ad-hoc 署名が必要。開発者証明書が無ければ Gatekeeper の警告は避けられない (どのエンジンでも同じ) |
| Linux   | tar.gz は容易。AppImage (squashfs) は後回し                                                      | 利用者の環境に WebKitGTK (WasmGC が動く版) が要る                                                                                                                                                                                                                                       |
| Web     | 静的ファイル (実装済み)                                                                          | COOP / COEP                                                                                                                                                                                                                                                                             |
| Android | ビルド済み APK + アセット → 署名し直し (Godot と同じ)。後で                                      | 署名                                                                                                                                                                                                                                                                                    |
| iOS     | Mac と Xcode が必須。どの方式でも避けられない                                                    | —                                                                                                                                                                                                                                                                                       |

## プレイヤーの実装の候補

| 案                                         | 長所                                                               | 短所                                                                                         |
| ------------------------------------------ | ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------- |
| **① WebView (Tauri / wry)**                | 今のエンジンがそのまま載る。数 MB。モバイルの道もある              | WebView の差。Linux は 3 件回避して動作済み、Windows / macOS は未検証                        |
| ② Chromium 同梱 (Electron / NW.js)         | 挙動が揃う。各 OS のビルド済み公式バイナリがある                   | 100 MB 超。① が駄目な OS だけの代替に                                                        |
| ③ Go ネイティブ (Ebitengine など)          | Ebitengine 2.10 で macOS・Linux も cgo 不要                        | **Go 製の wasm ランタイム wazero が WasmGC 未対応** で Biwa の出力が動かない。UI・組版も自作 |
| ④ Rust ネイティブ (SDL3 / wgpu + wasmtime) | wasmtime 47 (2026-07) で GC が既定で有効になり、ランタイム面は成立 | レイアウト・日本語組版・動画を自作。Web 版と別エンジンになる                                 |

**① を本命**、駄目な OS だけ ②。④ は性能や WebView の差が埋まらないとはっきりしたときの選択肢。
③ はクロスビルドの容易さが B の形では効かず、GC の欠如が重いので見送り。

## 段階

1. 今の Tauri のビルドで Windows / macOS の実機検証 (利用者に依頼中)。
   そのまま ① のプレイヤーの検証にもなる。
2. **汎用プレイヤーのスパイク** (着手 2026-10-12)
   - **進捗 (2026-10-13)**: プレイヤー・`biwa build --platform windows,macos,linux,web`・Actions のワークフロー
     (`.github/workflows/player.yml`) を実装。Linux ではプレイヤーのビルドから配布物の起動まで確認済み。
     Windows / macOS は、この Linux の上で仮の実行ファイルを使って組み立て (macOS は ad-hoc 署名の形式も) を確認。
     次は、利用者に Actions を走らせてもらい、成果物で本物の配布物を組み立てて各 OS の実機で確かめる。
     詳細は `docs/desktop-build.md`。
   - プレイヤー: 実行ファイルの横 (macOS は `.app/Contents/Resources/`) の `game/` を配信し、COOP / COEP を付ける。
     ゲームを焼き込まないので、一度ビルドすればどのゲームにも使える。
   - GitHub Actions の 3 OS でプレイヤーをビルドする (**ワークフローの実行と成果物の取得は利用者に依頼する**)。
   - `biwa build` は、ビルド済みプレイヤーから Windows / macOS / Linux 向けの配布物を、どの OS の上でも組み立てる。
     macOS は `apple-codesign` で ad-hoc 署名する。
   - 確かめること: この Linux から作った 3 OS 分の配布物が、各 OS の実機で動くか。
3. エンジンの JS をビルド済みにし、開発サーバを `biwa` に内蔵する (Node.js も不要にする)。
4. `--platform` を `windows|macos|linux|web` に分ける (段階 2 で着手)。

## 不確かな点

- Windows / macOS の WebView での動作 (特に macOS の独自スキームで SharedArrayBuffer が出るか)。
- `apple-codesign` の ad-hoc 署名で、組み立てた `.app` が Apple Silicon で起動するか。
  ダウンロードしたもの (quarantine 付き) を開くときの手順 (Sequoia 以降は「システム設定 → プライバシーとセキュリティ」から許可)。
- Linux の WebKitGTK の版のばらつき。
- WebView2 は Windows 10/11 にしか無い (7/8 は対象外)。

## 参考

- Wasmtime 47 で GC が既定で有効: https://newreleases.io/project/github/bytecodealliance/wasmtime/release/v47.0.0
- wazero は GC 未対応 (wazero#2483)
- Ebitengine 2.10 で cgo 不要: https://ebitengine.org/en/documents/2.10.html
- rcodesign: https://gregoryszorc.com/docs/apple-codesign/0.12.0/index.html
- Godot の macOS 書き出し (rcodesign): https://docs.godotengine.org/en/4.7/tutorials/export/exporting_for_macos.html
- electron-builder の複数プラットフォームビルド (Windows のアイコンに Wine が要る): https://www.electron.build/v26/docs/features/multi-platform-build
