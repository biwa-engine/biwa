# Biwa Engine: 配布形態 (デスクトップ・Web・モバイル) とメディア (サウンド・動画・フォント) の方針

検討日: 2026-10-11。

**進捗 (2026-10-11)**: Tauri v2 でビルドできる環境を作った (`biwa build`。既定はデスクトップ版、`--platform web` で Web の静的ビルド)。
Linux (WebKitGTK 2.52) ではゲームが動くことを確認済み。Windows / macOS は実機での検証待ち。
手順・判明したこと・残りは、リポジトリの `docs/desktop-build.md` に書いた。

## 背景

- 目指す配布形態の優先度:
  1. Windows / macOS / Linux のネイティブ (デスクトップアプリ)
  2. Web でのブラウザプレイ
  3. iOS / Android (2. ができれば無料配布では困らないので優先度は低い)
- 開発環境 (`biwa dev`) は、ホットリロードでき、どこでも軽く立つことが大事なので、Node.js + Vite のまま変えない。
  Node.js に乗っているので 2. は近い。
- 問題は 1.。ブラウザの抽象化に乗るか (Tauri などのブラウザベースの包み方)、ネイティブで書くか (Rust + SDL + wasmtime など)。
- Biwa Language の Tier1 が Wasm なのは、Wasm ランタイムに言語を隔離し、syscall さえ満たせばエンジン本体は別物でよい、という分離のため。
  ターゲットごとに適した技術でエンジンを書くことは当初から可能性に入れている。
- dev モードで未対応のメディア: サウンド・動画・フォント。

## 結論

**当面はエンジンを Web スタックの 1 本に保ち、デスクトップはそれを Tauri で包む。**
ネイティブでの書き直しは、Wasm の syscall 境界を保ったまま「将来の選択肢」として残す。

## 判断の決め手: UI が DOM/CSS の上にある

- いまのエンジンで重いのは描画ではなくレイアウトと文字である。
  - UI Element は DOM と CSS (vw / vh / percent、margin / padding、グリッド、Layers の重ね、`font-family`、文字の折り返し) にそのまま乗っている。
  - Message Window の文字送りも DOM である。
- SDL 系のネイティブで同じことをするには、一式を自前で揃えることになる:
  - レイアウトエンジン (Rust なら taffy)
  - 日本語の組版 (禁則処理・縦書き・ルビは今後必ず欲しくなる) とシェイピング (cosmic-text / parley など)
  - フォントの描画、動画のデコード
- さらに、開発時 (ブラウザ) と配布物 (ネイティブ) で別のエンジンが動くことになり、
  「dev では正しいのに配布版だけ崩れる」が構造的に起きる。ノベルゲームは見た目の一致がほぼすべてなので、これが最大のリスク。
- ブラウザの抽象化は、ノベルゲームが要るもの (組版・フォント・動画・音・レイアウト) がいちばん厚い部分であり、乗り続ける価値が大きい。

## 配布形態ごとの案

| 形態            | 案                                    | 備考                                                                                                                    |
| --------------- | ------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| 2. Web          | `vite build` の静的出力をそのまま配信 | ほぼ今の延長。cross-origin isolation の HTTP ヘッダの設定が要る                                                         |
| 1. デスクトップ | **Tauri v2** (OS の WebView)          | 本体が数 MB・メモリも軽い。macOS は WKWebView、Windows は WebView2、Linux は WebKitGTK                                  |
| 1. の代替       | Electron / NW.js (Chromium を同梱)    | 100 MB 超になるが、dev (Chrome) と同じエンジンで動く。RPGツクール MV/MZ (NW.js) や TyranoScript (Electron) の前例がある |
| 3. モバイル     | Tauri v2 の mobile                    | 優先度が低いので、同じ構成で後から試す                                                                                  |

- Tauri を第一候補にするのは、軽さに加えて、同じ包み方でモバイルの道も残るため。
- ただし **WebView の差** がリスク (次節で確かめる)。

## 最初にやるべき検証 (1〜2 日程度)

`~/test1` の `vite build` 出力を Tauri で包み、3 OS の WebView で確かめる。
結果次第で「Linux だけ Electron」「全部 Electron」を決める。エンジンのコードは包み方によらず同じ。

1. **WasmGC・externref・funcref が動くか** (Biwa の生成物は WasmGC)。
   - Safari は 18.2 で WasmGC に対応したはず。WKWebView は OS の版に連動するので、対応する macOS の下限が決まる。
   - **WebKitGTK (Linux) の対応版と、各ディストリビューションでの普及状況は不確か。実機で確かめる。**
     ここが落ちたら Linux は Electron が妥当。
   - → WebKitGTK 2.52 (Ubuntu 24.04) では動いた。ただし SharedArrayBuffer は
     `JSC_useSharedArrayBuffer=1` が要り、`Module.imports()` が GC の型で投げる (どちらも対処済み)。
     古い版での下限はまだ。AppImage なら WebKitGTK を同梱するので版の問題は避けられる (約 80 MB)。
2. Worker (Wasm を走らせている) と、cross-origin isolation (SharedArrayBuffer を使うなら)。
3. PixiJS の WebGL の性能 (WebKitGTK は弱いことがある)。
4. 動画のコーデックと音の自動再生。

## 未対応のメディア (どの包み方でも同じ設計で済む)

### サウンド

- Web Audio API で実装する。BGM・SE・ボイスのチャンネル、ループ、クロスフェード、音量。
- syscall は Canvas オブジェクトと同じ形にする: id を返す、遷移を積む、`sync` で文字送りと結ぶ、クリックで完了させる。
- 音量の設定は std の Config が解決して絶対値で渡す (エンジンは設定を持たない、の方針どおり)。
- 形式は、どこでも鳴る AAC (m4a) か MP3 を基本にし、Opus はビルド時の変換として選べるようにするのが安全。
- 自動再生の制限は、タイトル画面の最初のクリック (Link / SceneStartButton) で AudioContext を起こせば自然に解ける。

### 動画

- `<video>` を UI Element にするか、Canvas の上のオブジェクト (PixiJS の VideoTexture) にする。
  どちらも、クリックで飛ばせることと文字送りとの同期を Canvas API と揃える。
- コーデックが WebView ごとに一番ばらつく。
  - WebKitGTK は GStreamer のプラグイン次第で、利用者の環境に依存する。
  - これを避けたいことは Electron を選ぶ強い理由になる。
- 「H.264 (mp4) と VP9 (webm) を両方出力し、実行時に再生できる方を選ぶ」をビルドに組み込むのが堅い。

### フォント

- 配布物には同梱し、`FontFace` で登録する。Google Fonts は dev と Web 配信の便宜に留める。
- 文字送りの前に読み込みを待つ (`document.fonts.load`)。
- **コンパイラは scene の地の文をすべて知っている** ので、ビルド時に使う文字だけへフォントを subset できる
  (日本語フォントの数 MB → 数百 KB)。
  プレイヤー名など実行時に決まる文字のために、補助として全体 (または JIS 第 1 水準) を持つ二段構えにする。

## ネイティブで書き直す道 (将来の選択肢)

- Wasm を Tier1 にした分離 (syscall さえ満たせばエンジンは別物でよい) は正しく、選択肢は残る。
- やるなら次の構成が候補:
  - **SDL2 ではなく SDL3** (2025 年に安定版。SDL2 は保守のみ) か、wgpu + winit
  - wasmtime (WasmGC への対応は進んでいるが、externref の文字列や funcref のハンドラを含めて要確認)
  - レイアウトは taffy、文字は cosmic-text / parley、音は kira、動画は ffmpeg または dav1d (ライセンスとサイズに注意)
- この道を選ぶ条件: 「ブラウザでは性能が足りない」または「WebView の差が埋まらない」がはっきりしたとき。
- それまでは、**ネイティブの余地を残すために今からできること** を守る:
  - syscall の ABI を base_engine に閉じ込める (std の整理で進めた方向)。
  - エンジンの UI を「DOM の便利さ」に頼りすぎない仕様にしておく。
    例: UI の property を CSS の生の文字列で受けない (`text_font` が `font-family` にそのまま入る件)。
  - メディアの API を Web 固有の概念 (DOM 要素など) で定義しない。

## 進め方

1. Tauri の検証 (上の 4 点を 3 OS の WebView で)。
2. 結果に応じて、デスクトップの包み方を決める (Tauri のみ / Linux だけ Electron / 全部 Electron)。
3. `biwa build --platform desktop|web` を作る (済。既定は desktop) (Vite の静的ビルドと、包み方ごとのバンドル)。macOS の署名・公証の手順もここで要る。
4. サウンド → フォント (subset 込み) → 動画、の順でメディアの syscall と std の API を足す。
   サウンドは影響が小さく価値が大きいので先、動画はコーデックの判断が要るので最後。

## 不確かな点 (確かめてから使うこと)

- WebKitGTK の WasmGC 対応版と、主要ディストリビューションに入っている版。
- 各 WebView での H.264 / VP9 / AV1・Opus の再生可否。
- wasmtime の WasmGC 対応の成熟度 (ネイティブの道を選ぶときだけ)。
- Safari の WasmGC 対応 (18.2) と、それが使える macOS の下限。
