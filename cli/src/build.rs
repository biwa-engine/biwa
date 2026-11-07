//! `biwa build`: 配布物のビルド。
//!
//!   - `windows` / `macos` / `linux`: ネイティブの runtime (`engine/desktop-rust`) に、
//!     ゲームのデータ (`game/` = `game.wasm` + `assets/` + `biwa-game.json`) を添えて配布物を組み立てる (`desktop.rs`)。
//!   - `windows-webview` / `macos-webview` / `linux-webview`: WebView の runtime (`engine/desktop-tauri`) に、
//!     Web 版と同じもの (+ WebView の検証ページ) をゲームのデータとして添える。
//!   - `desktop` (既定): `biwa` が動いている OS 向けのネイティブ版。
//!   - `web`: ビルド済みの Web runtime (`engine/nodejs` の `npm run build:runtime`) に、`game.wasm` +
//!     `assets/` + `biwa-game.json` を添えて `dist/web/` に出す。HTTP で配信する。
//!
//! どれもツールチェインのビルド済みの runtime (`toolchain.rs`) にゲームのデータを添えるだけで、
//! runtime のコンパイルも Vite (Node.js) も走らない。どの OS の上でも全ターゲット向けに作れる。
//!
//! 既定が `desktop` なのは、配布形態の第一の目標がデスクトップアプリだからである。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use biwac_base::Target;
use clap::{Args, ValueEnum};

use crate::assets::{self, Probe};
use crate::project::Project;
use crate::toolchain::{DesktopOs, DesktopTarget, Runtime, Toolchain};
use crate::{desktop, dev, files};

#[derive(Debug, Args)]
pub struct BuildArgs {
    /// 対象の biwa パッケージ (省略時はカレントディレクトリから探す)
    #[arg(short, long, value_name = "DIR")]
    pub package_path: Option<PathBuf>,

    /// キャッシュを無視して最初から建て直す
    #[arg(short, long)]
    pub rebuild: bool,

    /// 配布形態 (カンマ区切りで複数)
    #[arg(long, value_enum, value_delimiter = ',', default_value = "desktop")]
    pub platform: Vec<Platform>,

    /// runtime を取るツールチェイン (省略時は環境変数 BIWA_TOOLCHAIN、なければこの CLI と同じもの。実験用)
    #[arg(long, value_name = "NAME")]
    pub toolchain: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Platform {
    /// `biwa` が動いている OS のデスクトップアプリ (ネイティブ)
    Desktop,
    /// Windows のデスクトップアプリ (ネイティブ、`dist/windows/`)
    Windows,
    /// macOS のデスクトップアプリ (ネイティブ、`dist/macos/`)
    Macos,
    /// Linux のデスクトップアプリ (ネイティブ、`dist/linux/`)
    Linux,
    /// Windows のデスクトップアプリ (WebView、`dist/windows-webview/`)
    WindowsWebview,
    /// macOS のデスクトップアプリ (WebView、`dist/macos-webview/`)
    MacosWebview,
    /// Linux のデスクトップアプリ (WebView、`dist/linux-webview/`)
    LinuxWebview,
    /// ブラウザで遊ぶ静的ファイル (`dist/web/`)
    Web,
}

/// 配布物のコード生成ターゲット。
///
/// どの runtime も wasm を走らせる。
const TARGET: Target = Target::Wasm;

/// ゲームのデータを作る場所 (`.biwa_build/` からの相対)。runtime の `game/` になる。
const GAME_DATA_DIR: &str = "game-data";
/// ゲームのデータの中の wasm
/// (`engine/desktop-rust/crates/player/src/options.rs`、`engine/nodejs/src/game/runtime-entry.ts`)。
const WASM_FILE: &str = "game.wasm";
/// ゲームのデータの中の、検証ページの置き場所。
const PROBE_DIR: &str = "__biwa_probe";
/// ゲームのデータの中の、runtime が読むゲームの情報 (`engine/desktop-tauri/src-tauri/src/main.rs`、
/// `engine/desktop-rust/crates/player/src/options.rs`、`engine/nodejs/src/game/runtime-entry.ts`)。
const GAME_INFO_FILE: &str = "biwa-game.json";

pub fn run(args: BuildArgs) -> Result<()> {
    let start_dir = args
        .package_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));
    let project = Project::discover(&start_dir)?;
    let (web, targets) = resolve_platforms(&args.platform)?;
    let webview = targets.iter().any(|t| t.runtime == Runtime::Webview);

    println!("Package: {} ({})", project.name, project.root.display());

    // runtime が揃っているかは、時間のかかるビルドの前に確かめる。
    let toolchain = Toolchain::select(args.toolchain.as_deref())?;
    println!("Toolchain: {}", toolchain.name);
    let runtimes = targets
        .iter()
        .map(|&target| toolchain.runtime(target).map(|path| (target, path)))
        .collect::<Result<Vec<_>>>()?;
    // Web 版と WebView 版は、どちらもビルド済みの Web runtime をゲームのデータの土台にする。
    let web_runtime = if web || webview {
        Some(toolchain.web_runtime()?)
    } else {
        None
    };

    files::ensure_std(&project)?;
    dev::compile(&project, args.rebuild)?;
    let wasm =
        project
            .generated_dir(TARGET)
            .join(format!("{}.{}", project.name, TARGET.bin_extension()));
    let game = GameData {
        project: &project,
        wasm: &wasm,
        toolchain: &toolchain.name,
    };

    let mut outputs = Vec::new();

    if let Some(web_runtime) = &web_runtime
        && web
    {
        let out_dir = project.dist_dir().join("web");
        game.build_web(&out_dir, web_runtime)?;
        outputs.push(out_dir);
    }

    let data_dir = project.build_dir().join(GAME_DATA_DIR);
    let mut native_data = None;
    let mut webview_data = None;
    for (target, runtime) in &runtimes {
        let game_data = match target.runtime {
            Runtime::Native => match &native_data {
                Some(dir) => dir,
                None => {
                    let dir = data_dir.join("native");
                    game.build_native(&dir)?;
                    native_data.insert(dir)
                }
            },
            Runtime::Webview => match &webview_data {
                Some(dir) => dir,
                None => {
                    let dir = data_dir.join("webview");
                    let web_runtime = web_runtime
                        .as_deref()
                        .expect("looked up above for WebView targets");
                    game.build_webview(&dir, web_runtime)?;
                    webview_data.insert(dir)
                }
            },
        };
        println!("Assembling the {} app", target.name());
        outputs.push(desktop::assemble(&project, *target, runtime, game_data)?);
    }

    println!();
    println!("Built:");
    for path in &outputs {
        println!("  {}", path.display());
    }
    if web {
        println!(
            "Serve dist/web over HTTP with `Cross-Origin-Opener-Policy: same-origin` and \
             `Cross-Origin-Embedder-Policy: require-corp` (needed for SharedArrayBuffer)."
        );
    }
    if webview {
        println!(
            "Run a WebView app with `--probe` (or BIWA_PROBE=1) to open the WebView probe page instead of the game."
        );
    }

    Ok(())
}

/// `--platform` を「Web を作るか」と「どのデスクトップ版を作るか」に分ける。
fn resolve_platforms(platforms: &[Platform]) -> Result<(bool, Vec<DesktopTarget>)> {
    use DesktopOs::{Linux, Macos, Windows};
    use Runtime::{Native, Webview};

    let mut web = false;
    let mut targets = Vec::new();
    for platform in platforms {
        let target = match platform {
            Platform::Web => {
                web = true;
                continue;
            }
            Platform::Desktop => DesktopTarget::new(DesktopOs::host()?, Native),
            Platform::Windows => DesktopTarget::new(Windows, Native),
            Platform::Macos => DesktopTarget::new(Macos, Native),
            Platform::Linux => DesktopTarget::new(Linux, Native),
            Platform::WindowsWebview => DesktopTarget::new(Windows, Webview),
            Platform::MacosWebview => DesktopTarget::new(Macos, Webview),
            Platform::LinuxWebview => DesktopTarget::new(Linux, Webview),
        };
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    Ok((web, targets))
}

/// ゲームのデータ (runtime の `game/`、または Web 版の出力そのもの) を作るのに要るもの。
struct GameData<'a> {
    project: &'a Project,
    /// コンパイラの出力。
    wasm: &'a Path,
    /// 組み立てに使うツールチェインの名前 (`biwa-game.json` に書く)。
    toolchain: &'a str,
}

impl GameData<'_> {
    /// ネイティブ版: wasm・アセット・ゲームの情報だけ。
    fn build_native(&self, out_dir: &Path) -> Result<()> {
        recreate_dir(out_dir)?;
        self.add_game(out_dir)
    }

    /// Web 版: ビルド済みの Web runtime (`engine/nodejs` の `npm run build:runtime`) に、wasm・アセット・
    /// ゲームの情報を添える。runtime はページの横の `biwa-game.json` と `game.wasm` を実行時に読む
    /// (`engine/nodejs/src/game/runtime-entry.ts`)。
    fn build_web(&self, out_dir: &Path, web_runtime: &Path) -> Result<()> {
        recreate_dir(out_dir)?;
        files::copy_dir(web_runtime, out_dir)?;
        self.add_game(out_dir)
    }

    /// WebView 版: Web 版に WebView の検証ページを足す。
    /// 検証ページは実際のゲームの wasm を WebView がコンパイルできるかも見るので、その写しも添える。
    fn build_webview(&self, out_dir: &Path, web_runtime: &Path) -> Result<()> {
        self.build_web(out_dir, web_runtime)?;
        let probe_dir = out_dir.join(PROBE_DIR);
        assets::extract::<Probe>(&probe_dir)?;
        copy_file(self.wasm, &probe_dir.join(WASM_FILE))
    }

    /// wasm・アセット・ゲームの情報を `out_dir` に置く。
    fn add_game(&self, out_dir: &Path) -> Result<()> {
        copy_file(self.wasm, &out_dir.join(WASM_FILE))?;
        files::copy_assets(self.project, out_dir)?;
        self.write_info(out_dir)
    }

    /// runtime が読むゲームの情報 (ウィンドウのタイトルなど) を書く。
    ///
    /// `buildId` は wasm の内容から決まる値で、Web 版がブラウザのキャッシュを避けるのに使う。
    /// `toolchain` はどのツールチェインで組み立てたか (runtime は読まない。不具合の報告で版を知るため)。
    fn write_info(&self, out_dir: &Path) -> Result<()> {
        let wasm = std::fs::read(self.wasm)
            .with_context(|| format!("failed to read {}", self.wasm.display()))?;
        let info = serde_json::json!({
            "name": self.project.name,
            "title": self.project.name,
            "version": self.project.version.as_deref().unwrap_or("0.1.0"),
            "buildId": files::content_id(&wasm),
            "toolchain": self.toolchain,
        });
        let info_path = out_dir.join(GAME_INFO_FILE);
        std::fs::write(&info_path, serde_json::to_string_pretty(&info)?)
            .with_context(|| format!("failed to write {}", info_path.display()))
    }
}

fn recreate_dir(dir: &Path) -> Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)
            .with_context(|| format!("failed to remove {}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))
}

fn copy_file(from: &Path, to: &Path) -> Result<()> {
    std::fs::copy(from, to)
        .map(|_| ())
        .with_context(|| format!("failed to copy {} to {}", from.display(), to.display()))
}
