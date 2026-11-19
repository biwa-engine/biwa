# ツールチェインのインストールと更新

issue #24。ゲームを作る人が Windows / macOS / Linux のどれか 1 つの上で、`biwa` を入れ、更新し、
どの OS 向けの配布物も作れるようにする仕組み。実装は `cli/src/manage/`、配布は `.github/workflows/release.yml`、
インストーラは `install/`。

## ツールチェイン

**ツールチェイン = CLI (コンパイラ・std を含む) + 全ターゲットの runtime** で、版の単位である。
コンパイラが出す wasm と runtime の間には約束 (syscall の import、UI の property の番号など。下の ABI) があるので、
runtime を CLI と別に選ぶことはしない。

名前は 2 通りある。

| 種類    | 名前                          | release のタグ        | チャンネル |
| ------- | ----------------------------- | --------------------- | ---------- |
| 版      | `0.1.0`                       | `v0.1.0`              | stable     |
| nightly | コミットの先頭 12 文字        | `nightly-<commit>`    | nightly    |

2026-11 の時点では版はまだ無く (unversioned)、nightly だけで運用する。版が出ても nightly は残す。
`biwa --version` は `0.1.0 (<commit>)` か `nightly (<commit>)` を出す。

## 置き場所

```
<home>/                                    ← 環境変数 BIWA_HOME、無ければ ~/.biwa (Windows は %USERPROFILE%\.biwa)
  bin/biwa[.exe]                           ← ランチャー
  toolchains/<name>/
    bin/biwa[.exe]                         ← そのツールチェインの CLI (実体)
    runtime/native-{windows-x86_64,macos-universal,linux-x86_64}/biwa-runtime[.exe]
    runtime/webview-{windows-x86_64,macos-universal,linux-x86_64}/biwa-runtime[.exe]
    runtime/web/                           ← ビルド済みの Web runtime (静的ファイル)
  settings.json                            ← { "default": "<name>" }
  env                                      ← sh 用の PATH の設定 (install.sh が書く)
```

全ターゲットの runtime を 1 度に入れる (1 つ 40〜50 MB)。どの OS の上でも全 OS 向けの配布物を作れるようにするためである。

## ランチャー

実行ファイルは 1 つで、置き場所で役割が変わる。

- `<home>/bin/biwa` に置かれているとき (ランチャー): 管理のコマンド (`toolchain` / `update` / `self`) は自分でこなし、
  それ以外 (`dev` / `build` / `publish` …) は選んだツールチェインの `toolchains/<name>/bin/biwa` に**引数を解釈せずに**渡す
  (ランチャーとツールチェインの版が違っても、ツールチェインの側の引数がそのまま通るように)。
- それ以外 (ツールチェインの中の `biwa`、cargo でビルドしたもの): 実体として動く。管理のコマンドも使える。

ツールチェインを選ぶ順:

1. `--toolchain <NAME>` (引数)、無ければ環境変数 `BIWA_TOOLCHAIN`
2. パッケージの `biwa-package.json` の `toolchain`
3. 既定 (`biwa toolchain default`)

1 で選んでも 2 の要件は外せない (満たさなければエラー)。

## `biwa-package.json` の `toolchain`

パッケージが要るツールチェイン。**最小要件**を書く。

```json
"toolchain": "0.1.0"                 // 0.1.0 以上 (= { "min": "0.1.0" })
"toolchain": { "min": "0.1.0" }
"toolchain": "3f2a9c1b7d4e"          // nightly: そのものが要る
```

- 版: 既定のツールチェインが満たせば既定、でなければインストール済みで満たす最新の版を使う。
- nightly (ハッシュ): 新旧を比べられないので、同じ名前のものが無ければすぐにエラー。nightly は版を満たさない。
- 満たすものがインストールされていなければエラー (自動では入れない)。
- ツールチェインの CLI を直接起動したときも照合する。手元でビルドした CLI (`-dirty`・`unknown`) は警告に留める。

まだ無いもの: 上限 (`max`)、満たすものの自動インストール、`edition` への改称 (検討中)。

## コマンド

```sh
biwa toolchain list                   # インストール済み (既定・この CLI・runtime だけのものに印)
biwa toolchain install [SPEC]         # SPEC: stable / nightly / 0.1.0 / <commit>。省略で最新の版 (無ければ最新の nightly)
biwa toolchain install --archive FILE # 手元の biwa-<name>-<host>.{tar.gz,zip} から
biwa toolchain default [NAME]         # 既定を見る・決める
biwa toolchain remove NAME            # 既定は消せない
biwa update                           # 既定のチャンネル (版なら stable、nightly なら nightly) の最新を入れて既定にし、ランチャーも替える
biwa self update                      # ランチャーだけを最新にする
```

- インストールは `<home>/tmp/` に落として展開し、`toolchains/<name>` を rename で置く (途中で止まっても半端なものが並ばない)。
- release の資産は `SHA256SUMS` で確かめる。
- Windows では実行中の exe を上書きできないので、ランチャーを替えるときは古いものを `biwa.exe.old` に改名して退かし、
  次に起動したときに消す。
- 一覧の取得先は `BIWA_RELEASES_URL` (既定 `https://api.github.com/repos/biwa-engine/biwa/releases`) で差し替えられる
  (試験用)。`GITHUB_TOKEN` があれば API に使う (認証なしは 1 時間に 60 回まで)。

## インストーラ

```sh
# macOS / Linux
curl -sSf https://raw.githubusercontent.com/biwa-engine/biwa/main/install/install.sh | sh
curl -sSf https://raw.githubusercontent.com/biwa-engine/biwa/main/install/install.sh | sh -s -- --toolchain nightly
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/biwa-engine/biwa/main/install/install.ps1 | iex
```

どちらも、このホスト向けの CLI (`biwa-cli-<name>-<host>`) を落として `SHA256SUMS` で確かめ、その CLI に
`biwa toolchain install <name> --default` をさせる (ツールチェインの取得・確認・展開・ランチャーの配置は CLI の仕事)。
そのあと `~/.biwa/bin` に PATH を通す (sh は `~/.biwa/env` を書いて `~/.profile` などから読む。`--no-modify-path` で止める。
Windows は利用者の環境変数 `Path`。`BIWA_NO_MODIFY_PATH=1` で止める)。
curl / irm で取ったファイルには macOS の quarantine が付かないので、Gatekeeper には止められない。

## 配布 (`.github/workflows/release.yml`)

- 手で実行 (workflow_dispatch) → nightly。タグ `nightly-<commit>` の prerelease を作る。
- タグ `v<major>.<minor>.<patch>` の push → 版の release。CLI は環境変数 `BIWA_RELEASE_VERSION` で版を名乗る (`cli/build.rs`)。
- 資産 (`<host>` は `windows-x86_64` / `macos-universal` / `linux-x86_64`):
  - `biwa-<name>-<host>.{tar.gz,zip}`: `<home>` にそのまま展開できる形 (`bin/biwa` と `toolchains/<name>/…`)
  - `biwa-cli-<name>-<host>[.exe]`: CLI だけ (`biwa self update` とインストーラが使う)
  - `SHA256SUMS`
- 同じ内容は Actions の成果物 `biwa-release` にも残る。

## ABI

生成物と runtime の約束の版。コンパイラが wasm の custom section `biwa.abi` に u32 (little endian) で書き
(`compiler/src/biwac_generator/src/arch/wasm.rs` の `ABI_VERSION`)、runtime が compile の前に照合する
(ネイティブ `engine/desktop-rust/crates/vm/src/abi.rs`、Web `engine/nodejs/src/engine/vm/wasm/abi.ts`)。
無い・合わないときは「同じツールチェインで作り直す」ように言って止まる。

**約束を変えたら 3 か所の `ABI_VERSION` を一緒に上げる** (std の `[[native(arch="wasm")]]` の import 宣言を変えたときも)。
いまは 1 つの版だけを受け付ける。runtime だけを独立に直して出したくなったら、受け付ける範囲を持たせる。

## まだやらないこと

コード署名 (Windows の Authenticode、macOS の Developer ID と公証)、パッケージマネージャ (Homebrew・winget・scoop)、
`toolchain` の上限と自動インストール、runtime の遅延ダウンロード、biwac の CLI からの切り離し、アンインストール
(`biwa self uninstall`)。
