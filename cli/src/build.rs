//! `biwa build`: 配布物のビルド。
//!
//! どの配布形態でも中身は同じ Web の静的ビルド (Vite) である。
//!   - `desktop` (既定): それを Tauri のシェルに入れて、OS ごとのアプリにする (`desktop_tauri.rs`)。
//!     デスクトップ向けの実装は今後増えうるが、当面 `desktop` は Tauri (`engine/desktop-tauri`) を指す。
//!   - `web`: それをそのまま `dist/web/` に出す。HTTP で配信する。
//!
//! 既定が `desktop` なのは、配布形態の第一の目標がデスクトップアプリだからである。

use std::path::PathBuf;

use anyhow::Result;
use biwac_base::Target;
use clap::{Args, ValueEnum};

use crate::project::Project;
use crate::{desktop_tauri, dev, runtime};

#[derive(Debug, Args)]
pub struct BuildArgs {
    /// 対象の biwa パッケージ (省略時はカレントディレクトリから探す)
    #[arg(short, long, value_name = "DIR")]
    pub package_path: Option<PathBuf>,

    /// キャッシュを無視して最初から建て直す
    #[arg(short, long)]
    pub rebuild: bool,

    /// 配布形態
    #[arg(long, value_enum, default_value_t = Platform::Desktop)]
    pub platform: Platform,

    /// デスクトップ版で作るインストーラの種類 (Tauri の bundle target、カンマ区切り)
    ///
    /// 省略時はその OS で作れるものすべて
    /// (Linux: deb, rpm, appimage / macOS: app, dmg / Windows: nsis, msi)。
    #[arg(long, value_name = "LIST", value_delimiter = ',')]
    pub bundles: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Platform {
    /// Windows / macOS / Linux のデスクトップアプリ (`dist/desktop/`)
    Desktop,
    /// ブラウザで遊ぶ静的ファイル (`dist/web/`)
    Web,
}

/// 配布物のコード生成ターゲット。
///
/// エンジンの Worker が走らせる wasm に固定する。
const TARGET: Target = Target::Wasm;

pub fn run(args: BuildArgs) -> Result<()> {
    let start_dir = args
        .package_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));
    let project = Project::discover(&start_dir)?;

    println!("Package: {} ({})", project.name, project.root.display());

    runtime::ensure_std(&project)?;
    runtime::ensure_engine(&project)?;
    // `public/assets` のリンク (`runtime::link_assets`) は張らない。
    // 静的ビルドはリンクを当てにせず、出力へ直接コピーする (`copy_assets`)。
    // Windows ではリンクの作成に開発者モードが要るので、配布物のビルドをそれに依らせない。

    dev::build(&project, TARGET, args.rebuild)?;

    match args.platform {
        Platform::Web => {
            let out_dir = project.dist_dir().join("web");
            build_web(&project, &out_dir)?;
            println!();
            println!("Built {}", out_dir.display());
            println!(
                "Serve it over HTTP with `Cross-Origin-Opener-Policy: same-origin` and \
                 `Cross-Origin-Embedder-Policy: require-corp` (needed for SharedArrayBuffer)."
            );
            Ok(())
        }
        Platform::Desktop => {
            desktop_tauri::ensure_shell(&project)?;
            let frontend_dir = desktop_tauri::frontend_dir(&project);
            build_web(&project, &frontend_dir)?;
            desktop_tauri::add_probe(&project, &frontend_dir)?;
            desktop_tauri::bundle(&project, &frontend_dir, &args.bundles)
        }
    }
}

/// Web の静的ビルドを `out_dir` に作る。
fn build_web(project: &Project, out_dir: &std::path::Path) -> Result<()> {
    runtime::vite_build(project, out_dir)?;
    runtime::copy_assets(project, out_dir)
}
