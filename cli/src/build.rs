//! `biwa build`: 配布物のビルド。
//!
//!   - `windows` / `macos` / `linux`: ネイティブのプレイヤー (`engine/desktop-rust`) に、
//!     ゲームのデータ (`game/` = `game.wasm` + `assets/`) を添えて配布物を組み立てる (`desktop.rs`)。
//!   - `windows-webview` / `macos-webview` / `linux-webview`: WebView のプレイヤー
//!     (`engine/desktop-tauri`) に、Web の静的ビルド (Vite) をゲームのデータとして添える。
//!   - `desktop` (既定): `biwa` が動いている OS 向けのネイティブ版。
//!   - `web`: Web の静的ビルドをそのまま `dist/web/` に出す。HTTP で配信する。
//!
//! デスクトップ版はビルド済みのプレイヤー (`player.rs`) を使い、コンパイルは走らないので、
//! どの OS の上でも全 OS 向けに作れる。
//!
//! 既定が `desktop` なのは、配布形態の第一の目標がデスクトップアプリだからである。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use biwac_base::Target;
use clap::{Args, ValueEnum};

use crate::assets::{self, Probe};
use crate::player::{self, DesktopOs, DesktopTarget, Runtime};
use crate::project::Project;
use crate::{desktop, dev, runtime};

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

    /// ビルド済みプレイヤーの置き場所 (省略時は環境変数 BIWA_PLAYERS_DIR、なければ ~/.biwa/players)
    #[arg(long, value_name = "DIR")]
    pub players_dir: Option<PathBuf>,
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
/// エンジンの Worker が走らせる wasm に固定する。
const TARGET: Target = Target::Wasm;

/// WebView 版のゲームのデータを作る場所 (`.biwa_runtime/` からの相対)。
const WEBVIEW_GAME_DATA_DIR: &str = "dist-game";
/// ネイティブ版のゲームのデータを作る場所 (`.biwa_runtime/` からの相対)。
const NATIVE_GAME_DATA_DIR: &str = "dist-game-native";
/// ネイティブ版のゲームのデータの中の wasm (`engine/desktop-rust/crates/player/src/options.rs`)。
const NATIVE_WASM_FILE: &str = "game.wasm";
/// ゲームのデータの中の、検証ページの置き場所。
const PROBE_DIR: &str = "__biwa_probe";
/// ゲームのデータの中の、プレイヤーが読むゲームの情報
/// (`engine/desktop-tauri/src-tauri/src/main.rs`、`engine/desktop-rust/crates/player/src/options.rs`)。
const GAME_INFO_FILE: &str = "biwa-game.json";

pub fn run(args: BuildArgs) -> Result<()> {
    let start_dir = args
        .package_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));
    let project = Project::discover(&start_dir)?;
    let (web, targets) = resolve_platforms(&args.platform)?;

    println!("Package: {} ({})", project.name, project.root.display());

    // プレイヤーが揃っているかは、時間のかかるビルドの前に確かめる。
    let players = if targets.is_empty() {
        Vec::new()
    } else {
        let dir = player::players_dir(args.players_dir.as_deref())?;
        targets
            .iter()
            .map(|&target| player::find(&dir, target).map(|path| (target, path)))
            .collect::<Result<Vec<_>>>()?
    };
    let webview = targets.iter().any(|t| t.runtime == Runtime::Webview);

    runtime::ensure_std(&project)?;
    // Vite (Node) が要るのは Web の静的ビルドを作るときだけ。ネイティブ版は wasm と素材を運ぶだけである。
    if web || webview {
        runtime::ensure_engine(&project)?;
    }
    // `public/assets` のリンク (`runtime::link_assets`) は張らない。
    // 静的ビルドはリンクを当てにせず、出力へ直接コピーする (`copy_assets`)。
    // Windows ではリンクの作成に開発者モードが要るので、配布物のビルドをそれに依らせない。

    dev::build(&project, TARGET, args.rebuild)?;

    let mut outputs = Vec::new();

    if web {
        let out_dir = project.dist_dir().join("web");
        build_web(&project, &out_dir)?;
        outputs.push(out_dir);
    }

    let mut native_data = None;
    let mut webview_data = None;
    for (target, player) in &players {
        let game_data = match target.runtime {
            Runtime::Native => match &native_data {
                Some(dir) => dir,
                None => {
                    let dir = project.runtime_dir().join(NATIVE_GAME_DATA_DIR);
                    build_native_game_data(&project, &dir)?;
                    native_data.insert(dir)
                }
            },
            Runtime::Webview => match &webview_data {
                Some(dir) => dir,
                None => {
                    let dir = project.runtime_dir().join(WEBVIEW_GAME_DATA_DIR);
                    build_webview_game_data(&project, &dir)?;
                    webview_data.insert(dir)
                }
            },
        };
        println!("Assembling the {} app", target.name());
        outputs.push(desktop::assemble(&project, *target, player, game_data)?);
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

/// Web の静的ビルドを `out_dir` に作る。
fn build_web(project: &Project, out_dir: &Path) -> Result<()> {
    runtime::vite_build(project, out_dir)?;
    runtime::copy_assets(project, out_dir)
}

/// ネイティブ版のゲームのデータ (プレイヤーの `game/` になるもの) を作る。
///
/// コンパイラが出した wasm と素材、ゲームの情報だけである。
fn build_native_game_data(project: &Project, out_dir: &Path) -> Result<()> {
    if out_dir.exists() {
        std::fs::remove_dir_all(out_dir)
            .with_context(|| format!("failed to remove {}", out_dir.display()))?;
    }
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;

    let wasm = runtime::placed_wasm(project);
    std::fs::copy(&wasm, out_dir.join(NATIVE_WASM_FILE))
        .with_context(|| format!("failed to copy {}", wasm.display()))?;
    runtime::copy_assets(project, out_dir)?;
    write_game_info(project, out_dir)
}

/// WebView 版のゲームのデータ (プレイヤーの `game/` になるもの) を作る。
///
/// Web の静的ビルドに、プレイヤーが読むゲームの情報と、WebView の検証ページを足す。
/// 検証ページは実際のゲームの wasm を WebView がコンパイルできるかも見るので、その写しも添える。
fn build_webview_game_data(project: &Project, out_dir: &Path) -> Result<()> {
    build_web(project, out_dir)?;
    write_game_info(project, out_dir)?;

    let probe_dir = out_dir.join(PROBE_DIR);
    assets::extract::<Probe>(&probe_dir)?;
    let wasm = runtime::placed_wasm(project);
    std::fs::copy(&wasm, probe_dir.join("game.wasm"))
        .with_context(|| format!("failed to copy {}", wasm.display()))?;

    Ok(())
}

/// プレイヤーが読むゲームの情報 (ウィンドウのタイトルなど) を書く。
fn write_game_info(project: &Project, out_dir: &Path) -> Result<()> {
    let info = serde_json::json!({
        "name": project.name,
        "title": project.name,
        "version": project.version.as_deref().unwrap_or("0.1.0"),
    });
    let info_path = out_dir.join(GAME_INFO_FILE);
    std::fs::write(&info_path, serde_json::to_string_pretty(&info)?)
        .with_context(|| format!("failed to write {}", info_path.display()))
}
