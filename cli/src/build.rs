//! `biwa build`: 配布物のビルド。
//!
//! どの配布形態でも中身は同じ Web の静的ビルド (Vite) である。
//!   - `windows` / `macos` / `linux`: それをゲームのデータ (`game/`) として、
//!     ビルド済みのプレイヤー (`player.rs`) に添えて配布物を組み立てる (`desktop.rs`)。
//!     コンパイルは走らないので、どの OS の上でも全 OS 向けに作れる。
//!   - `desktop` (既定): `biwa` が動いている OS を指す。
//!   - `web`: 静的ビルドをそのまま `dist/web/` に出す。HTTP で配信する。
//!
//! 既定が `desktop` なのは、配布形態の第一の目標がデスクトップアプリだからである。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use biwac_base::Target;
use clap::{Args, ValueEnum};

use crate::assets::{self, Probe};
use crate::player::{self, DesktopOs};
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
    /// `biwa` が動いている OS のデスクトップアプリ
    Desktop,
    /// Windows のデスクトップアプリ (`dist/windows/`)
    Windows,
    /// macOS のデスクトップアプリ (`dist/macos/`)
    Macos,
    /// Linux のデスクトップアプリ (`dist/linux/`)
    Linux,
    /// ブラウザで遊ぶ静的ファイル (`dist/web/`)
    Web,
}

/// 配布物のコード生成ターゲット。
///
/// エンジンの Worker が走らせる wasm に固定する。
const TARGET: Target = Target::Wasm;

/// デスクトップ版のゲームのデータを作る場所 (`.biwa_runtime/` からの相対)。
const GAME_DATA_DIR: &str = "dist-game";
/// ゲームのデータの中の、検証ページの置き場所。
const PROBE_DIR: &str = "__biwa_probe";
/// ゲームのデータの中の、プレイヤーが読むゲームの情報 (`engine/desktop-tauri/src-tauri/src/main.rs`)。
const GAME_INFO_FILE: &str = "biwa-game.json";

pub fn run(args: BuildArgs) -> Result<()> {
    let start_dir = args
        .package_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));
    let project = Project::discover(&start_dir)?;
    let (web, oses) = resolve_platforms(&args.platform)?;

    println!("Package: {} ({})", project.name, project.root.display());

    // プレイヤーが揃っているかは、時間のかかるビルドの前に確かめる。
    let players = if oses.is_empty() {
        Vec::new()
    } else {
        let dir = player::players_dir(args.players_dir.as_deref())?;
        oses.iter()
            .map(|&os| player::find(&dir, os).map(|path| (os, path)))
            .collect::<Result<Vec<_>>>()?
    };

    runtime::ensure_std(&project)?;
    runtime::ensure_engine(&project)?;
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

    if !players.is_empty() {
        let game_data = project.runtime_dir().join(GAME_DATA_DIR);
        build_game_data(&project, &game_data)?;
        for (os, player) in &players {
            println!("Assembling the {} app", os.name());
            outputs.push(desktop::assemble(&project, *os, player, &game_data)?);
        }
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
    if !players.is_empty() {
        println!(
            "Run the app with `--probe` (or BIWA_PROBE=1) to open the WebView probe page instead of the game."
        );
    }

    Ok(())
}

/// `--platform` を「Web を作るか」と「どの OS のデスクトップ版を作るか」に分ける。
fn resolve_platforms(platforms: &[Platform]) -> Result<(bool, Vec<DesktopOs>)> {
    let mut web = false;
    let mut oses = Vec::new();
    for platform in platforms {
        let os = match platform {
            Platform::Web => {
                web = true;
                continue;
            }
            Platform::Desktop => DesktopOs::host()?,
            Platform::Windows => DesktopOs::Windows,
            Platform::Macos => DesktopOs::Macos,
            Platform::Linux => DesktopOs::Linux,
        };
        if !oses.contains(&os) {
            oses.push(os);
        }
    }
    Ok((web, oses))
}

/// Web の静的ビルドを `out_dir` に作る。
fn build_web(project: &Project, out_dir: &Path) -> Result<()> {
    runtime::vite_build(project, out_dir)?;
    runtime::copy_assets(project, out_dir)
}

/// デスクトップ版のゲームのデータ (プレイヤーの `game/` になるもの) を作る。
///
/// Web の静的ビルドに、プレイヤーが読むゲームの情報と、WebView の検証ページを足す。
/// 検証ページは実際のゲームの wasm を WebView がコンパイルできるかも見るので、その写しも添える。
fn build_game_data(project: &Project, out_dir: &Path) -> Result<()> {
    build_web(project, out_dir)?;

    let info = serde_json::json!({
        "name": project.name,
        "title": project.name,
        "version": project.version.as_deref().unwrap_or("0.1.0"),
    });
    let info_path = out_dir.join(GAME_INFO_FILE);
    std::fs::write(&info_path, serde_json::to_string_pretty(&info)?)
        .with_context(|| format!("failed to write {}", info_path.display()))?;

    let probe_dir = out_dir.join(PROBE_DIR);
    assets::extract::<Probe>(&probe_dir)?;
    let wasm = runtime::placed_wasm(project);
    std::fs::copy(&wasm, probe_dir.join("game.wasm"))
        .with_context(|| format!("failed to copy {}", wasm.display()))?;

    Ok(())
}
