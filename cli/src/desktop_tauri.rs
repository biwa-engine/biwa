//! デスクトップ版 (Tauri): Web の静的ビルドを Tauri v2 のシェルに入れて、OS ごとのアプリにする。
//!
//! シェル (`engine/desktop-tauri`) はゲームに依らない共通の Tauri プロジェクトで、
//! `<project>/.biwa_desktop_tauri/` に展開して使う。ゲームごとの値 (名前・識別子・版・
//! 静的ビルドの場所) は `tauri build --config` で外から重ねる。
//!
//! Tauri はクロスコンパイルをほぼ支援しないので、各 OS のアプリはその OS の上でビルドする。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::assets::{self, DesktopTauri};
use crate::project::Project;
use crate::runtime;

/// シェルを展開したときの同一性を記録するファイル。
const DESKTOP_STAMP_FILE: &str = ".biwa-desktop-stamp";
/// 静的ビルドを置くディレクトリ (`.biwa_desktop_tauri/` からの相対)。
const FRONTEND_DIR: &str = "frontend";
/// ゲームごとの設定を書き出すファイル (`.biwa_desktop_tauri/` からの相対)。
const CONFIG_FILE: &str = "biwa.tauri.conf.json";
/// 検証ページの置き場所 (シェルからの相対と、静的ビルドの中での位置)。
const PROBE_SRC_DIR: &str = "probe";
const PROBE_DST_DIR: &str = "__biwa_probe";
/// Tauri の CLI の実体 (`node_modules/` からの相対)。
const TAURI_SCRIPT: &str = "@tauri-apps/cli/tauri.js";
/// シェルの Rust 側のディレクトリと、そこでの実行ファイル名 (`src-tauri/Cargo.toml`)。
const TAURI_DIR: &str = "src-tauri";
const SHELL_BINARY: &str = "biwa-desktop";

/// シェルを `.biwa_desktop_tauri/` に展開し、必要なら Tauri の CLI をインストールする。
pub fn ensure_shell(project: &Project) -> Result<()> {
    let dir = project.desktop_tauri_dir();
    let stamp_path = dir.join(DESKTOP_STAMP_FILE);
    let stamp = assets::stamp::<DesktopTauri>();

    if std::fs::read_to_string(&stamp_path).ok().as_deref() != Some(stamp.as_str()) {
        println!("Placing desktop shell into {}", dir.display());
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
        assets::extract::<DesktopTauri>(&dir)?;
        std::fs::write(&stamp_path, &stamp)
            .with_context(|| format!("failed to write {}", stamp_path.display()))?;
    }

    if !dir.join("node_modules").is_dir() {
        runtime::npm_install(&dir)?;
    }

    Ok(())
}

/// デスクトップ版に入れる静的ビルドの置き場所。
///
/// `dist/web/` とは分ける。検証ページなど、デスクトップ版にだけ入れるものがあるため。
pub fn frontend_dir(project: &Project) -> PathBuf {
    project.desktop_tauri_dir().join(FRONTEND_DIR)
}

/// 静的ビルドに WebView の検証ページを足す。
///
/// アプリを `--probe` 付きで起動すると、ゲームの代わりにこれが開く (`src-tauri/src/main.rs`)。
/// 実際のゲームの wasm を WebView がコンパイルできるかも見るので、それも添える。
pub fn add_probe(project: &Project, frontend_dir: &Path) -> Result<()> {
    let dst = frontend_dir.join(PROBE_DST_DIR);
    runtime::copy_dir(&project.desktop_tauri_dir().join(PROBE_SRC_DIR), &dst)?;

    let wasm = runtime::placed_wasm(project);
    std::fs::copy(&wasm, dst.join("game.wasm"))
        .with_context(|| format!("failed to copy {}", wasm.display()))?;

    Ok(())
}

/// `tauri build` でアプリとインストーラを作り、`dist/desktop/` に集める。
pub fn bundle(project: &Project, frontend_dir: &Path, bundles: &[String]) -> Result<()> {
    let dir = project.desktop_tauri_dir();
    let config_path = dir.join(CONFIG_FILE);
    let config = serde_json::json!({
        "productName": project.name,
        "version": project.version.as_deref().unwrap_or("0.1.0"),
        "identifier": identifier(&project.name),
        "build": {
            "frontendDist": frontend_dir,
        },
    });
    std::fs::write(&config_path, serde_json::to_string_pretty(&config)?)
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    // 前回の (名前や版が違うかもしれない) インストーラを拾わないように消しておく。
    let release_dir = dir.join(TAURI_DIR).join("target").join("release");
    let bundle_dir = release_dir.join("bundle");
    if bundle_dir.exists() {
        std::fs::remove_dir_all(&bundle_dir)
            .with_context(|| format!("failed to remove {}", bundle_dir.display()))?;
    }

    println!("Building the desktop app with Tauri (the first build takes a few minutes)");
    let mut command = runtime::node_script(&dir, TAURI_SCRIPT)?;
    command.arg("build").arg("--config").arg(&config_path);
    if !bundles.is_empty() {
        command.arg("--bundles").arg(bundles.join(","));
    }
    let status = command
        .status()
        .context("failed to run `tauri build` (is Node.js installed?)")?;
    if !status.success() {
        bail!("`tauri build` failed");
    }

    collect(project, &release_dir, &bundle_dir)
}

/// 出来たものを `dist/desktop/` に集める。
///
/// インストーラ類に加えて、インストールせずに試せるよう実行ファイルそのものも置く。
fn collect(project: &Project, release_dir: &Path, bundle_dir: &Path) -> Result<()> {
    let out_dir = project.dist_dir().join("desktop");
    if out_dir.exists() {
        std::fs::remove_dir_all(&out_dir)
            .with_context(|| format!("failed to remove {}", out_dir.display()))?;
    }
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;

    let mut outputs = Vec::new();

    let binary = release_dir.join(format!("{SHELL_BINARY}{}", std::env::consts::EXE_SUFFIX));
    if binary.is_file() {
        let dst = out_dir.join(format!("{}{}", project.name, std::env::consts::EXE_SUFFIX));
        std::fs::copy(&binary, &dst)
            .with_context(|| format!("failed to copy {}", binary.display()))?;
        outputs.push(dst);
    }

    // bundle/<種類>/<成果物>。種類ごとの作業用ディレクトリ (AppImage の AppDir など) は除く。
    if bundle_dir.is_dir() {
        for kind in std::fs::read_dir(bundle_dir)? {
            let kind = kind?.path();
            if !kind.is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(&kind)? {
                let path = entry?.path();
                let Some(name) = path.file_name() else {
                    continue;
                };
                let dst = out_dir.join(name);
                if is_installer(&path) {
                    std::fs::copy(&path, &dst)
                        .with_context(|| format!("failed to copy {}", path.display()))?;
                } else if path.extension().is_some_and(|e| e == "app") && path.is_dir() {
                    runtime::copy_dir(&path, &dst)?;
                } else {
                    continue;
                }
                outputs.push(dst);
            }
        }
    }

    println!();
    println!("Built:");
    for path in outputs {
        println!("  {}", path.display());
    }
    println!(
        "Run the app with `--probe` (or BIWA_PROBE=1) to open the WebView probe page instead of the game."
    );

    Ok(())
}

fn is_installer(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e, "deb" | "rpm" | "AppImage" | "msi" | "exe" | "dmg"))
}

/// アプリの識別子 (macOS の bundle identifier など)。
///
/// Tauri が許すのは英数字・`-`・`.` だけなので、それ以外は `-` に寄せる。
fn identifier(package_name: &str) -> String {
    let name: String = package_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("dev.biwa.game.{name}")
}
