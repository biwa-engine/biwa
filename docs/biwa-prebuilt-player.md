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
     (`.github/workflows/player.yml`、いまは `player-webview.yml`) を実装。Linux ではプレイヤーのビルドから配布物の起動まで確認済み。
     Windows / macOS は、この Linux の上で仮の実行ファイルを使って組み立て (macOS は ad-hoc 署名の形式も) を確認。
     詳細は `docs/desktop-build.md`。
   - **進捗 (2026-10-14)**: Actions で 3 OS のプレイヤーのビルドに成功 (run 38347793801)。
     その成果物から、この Linux の上で `~/test1` の 3 OS 分の配布物を組み立てた
     (macOS は universal の両アーキテクチャに ad-hoc 署名)。CI 製の Linux 版は起動しシーンまで進む。
     次は、Windows / macOS の実機での起動の確認 (利用者に依頼)。
   - **進捗 (2026-10-22〜26)**: Windows は実機で動いた (SmartScreen の警告のみ)。
     macOS は Safari を 18.2 以上にする必要があり (WasmGC)、さらに WKWebView は独自スキームで
     SharedArrayBuffer を出さないので、macOS ではプレイヤーが localhost (127.0.0.1、パッケージ名から決まるポート) で
     配るのを既定にした。更新したプレイヤーでの macOS の実機確認が次 (CI の再実行は利用者に依頼)。
   - プレイヤー: 実行ファイルの横 (macOS は `.app/Contents/Resources/`) の `game/` を配信し、COOP / COEP を付ける。
     ゲームを焼き込まないので、一度ビルドすればどのゲームにも使える。
   - GitHub Actions の 3 OS でプレイヤーをビルドする (**ワークフローの実行と成果物の取得は利用者に依頼する**)。
   - `biwa build` は、ビルド済みプレイヤーから Windows / macOS / Linux 向けの配布物を、どの OS の上でも組み立てる。
     macOS は `apple-codesign` で ad-hoc 署名する。
   - 確かめること: この Linux から作った 3 OS 分の配布物が、各 OS の実機で動くか。
3. エンジンの JS をビルド済みにし、開発サーバを `biwa` に内蔵する (Node.js も不要にする)。
4. `--platform` を `windows|macos|linux|web` に分ける (段階 2 で済み。`desktop` はホスト OS)。
5. **④ Rust ネイティブのプレイヤー** (`engine/desktop-rust`。2026-10 末〜。段階ごとの記録は `engine/desktop-rust/README.md`)
   - **進捗 (2026-11-02)**: 既定のデスクトップ版をネイティブに切り替えた。
     - `--platform windows|macos|linux` (と既定の `desktop`) はネイティブのプレイヤー、
       `--platform windows-webview|macos-webview|linux-webview` は ① の Tauri のプレイヤーを使う。
     - プレイヤーの置き場所の id は、ネイティブが `windows-x86_64` / `macos-universal` / `linux-x86_64`、
       Tauri が `windows-webview-x86_64` / `macos-webview-universal` / `linux-webview-x86_64`
       (それまで Tauri が使っていた id をネイティブに譲った)。
     - Actions は `.github/workflows/player-native.yml` (ネイティブ) と `player-webview.yml` (Tauri。旧 `player.yml`)。
     - ネイティブ版の `game/` は `game.wasm` + `assets/` + `biwa-game.json` だけで、Node.js (Vite) は要らない。
     - 組み立て (zip / tar.gz / .app と ad-hoc 署名) は ① と共通。
     - Linux (Xvfb + lavapipe) で配布物の起動まで確認済み。次は Actions でのビルドと Windows / macOS の実機確認 (段階 6)。
   - **進捗 (2026-11-07)**: 3 OS の実機で動作を確認し、WebView 版と同等の機能になったので main にマージした。
6. **ツールチェイン単位のバージョン管理と配布** (方針は `~/memo/biwa-toolchain-versioning-and-release.md`)
   - ツールチェイン = CLI (コンパイラ・std・エンジンの JS) + 全ターゲットの runtime。runtime を単独では選ばない
     (コンパイラが出す wasm と runtime の間に約束があるため)。バージョン番号が無い今は、名前はコミットの先頭 12 文字。
   - 置き場所は `~/.biwa` (`BIWA_HOME`): `bin/biwa`、`toolchains/<toolchain>/runtime/<runtime-id>/biwa-runtime`、
     いずれ `packages/` (パッケージの clone とビルドキャッシュ)。`~/.biwa/bin` にパスを通す。
   - **段階 1 (2026-11-07 実装)**:
     - 「プレイヤー」を「runtime」に改名した (実行ファイル `biwa-runtime`、id `native-*` / `webview-*`、
       成果物 `biwa-runtime-<id>`、Tauri の identifier `dev.biwa.runtime`)。
     - CLI はビルドしたコミットを焼き込み (`cli/build.rs`、`biwa --version`)、同じ名前のツールチェインの runtime を使う
       (`cli/src/toolchain.rs`。実験用に `--toolchain` / `BIWA_TOOLCHAIN`)。`biwa-game.json` にツールチェインの名前を書く。
     - Actions: `runtime-native.yml` / `runtime-webview.yml` (旧 `player-*.yml`) を `workflow_call` でも呼べるようにし、
       `release.yml` (workflow_dispatch) が CLI 3 OS 分と runtime 6 種類を同じコミットからビルドして、
       ホストの OS ごとに `~/.biwa` にそのまま展開できるアーカイブ (`biwa-toolchain-<host>`) を作る。
   - **Node.js を無くす (2026-11-07 合意)**: ゲームを作る人の作業から Node.js を無くす。Web 版 (Tier 1) も
     ビルド済みの Web runtime で組み立てる。Vite / Node はエンジンの開発者と CI だけの道具にする。
     1. ネイティブ版の dev モード (`biwa dev` の既定) — 済。
     2. Web runtime のビルド済み化 — 済。`engine/nodejs` の `npm run build:runtime` (`runtime` モード) が、ゲームを
        焼き込まず `biwa-game.json` と `game.wasm` を実行時に読む Web runtime を作り、CI (`runtime-web.yml`) がツールチェインの
        `runtime/web/` に入れる。`biwa build --platform web` と WebView 版はそれにゲームのデータを添えるだけ。
     3. `biwa dev --platform web` を CLI 内蔵の HTTP サーバ + 再読み込みにする — 済 (`cli/src/serve.rs`。
        建て直しは Server-Sent Events で知らせ、Web runtime は `biwa-game.json` の `dev: true` のときだけ聞く)。
        従来の Vite の開発サーバは `--vite` で残した (エンジンの開発と `--target typescript` のため)。
     4. ゲームを作る人の経路から Vite と `.biwa_runtime/` を外す。
   - 次: wasm に ABI のバージョンを入れて runtime で照合する (段階 2)。その後、ランチャー
     (`bin/biwa` が `toolchains/<toolchain>/bin/biwa` に渡す)、`biwa toolchain …` / `biwa self update`、
     GitHub Releases とインストールスクリプト (段階 3)。

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
