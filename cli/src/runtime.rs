//! エンジン (Node.js 版) の配置と、コンパイル結果の受け渡し。
//!
//! biwac の出力は `<project>/.biwa_build/typescript/` に生えるが、
//! Vite が見るのは `<project>/.biwa_runtime/` である。
//! ここが両者をつなぐ。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::assets::{self, Engine, Std};
use crate::project::Project;

/// エンジンを展開したときの同一性を記録するファイル。
const ENGINE_STAMP_FILE: &str = ".biwa-engine-stamp";
/// std を展開したときの同一性を記録するファイル。
///
/// このファイルが無い `deps/std` は「利用者が自分で用意したもの」とみなし、
/// CLI は一切触らない (リポジトリのシンボリックリンクなど)。
const STD_STAMP_FILE: &str = ".biwa-std-stamp";

/// 生成物とスタブを置くディレクトリ (`.biwa_runtime/` からの相対)。
const GAME_DIR: &str = "src/game";
/// エンジンが import するスタブ。ファイル名も関数名もエンジンと合意済みの規約。
const ENTRY_FILE: &str = "entry.ts";
/// コンパイラがエントリポイントに付ける名前。
const ENTRYPOINT_NAME: &str = "__biwa_entrypoint";

/// 依存パッケージとして `std` を用意する。
///
/// biwac は依存が `<root>/.biwa_build/deps/<name>/` に取得済みであることを前提とする。
/// 取得の実装はまだ無いので、同梱している std をここに置くのが CLI の仕事になる。
pub fn ensure_std(project: &Project) -> Result<()> {
    let deps_dir = project.dependencies_dir();
    let std_dir = deps_dir.join("std");
    let stamp_path = std_dir.join(STD_STAMP_FILE);
    let stamp = assets::stamp::<Std>();

    if std_dir.exists() {
        if !stamp_path.is_file() {
            // 利用者が自分で置いたもの。バージョンの管理も利用者の責任。
            return Ok(());
        }
        if std::fs::read_to_string(&stamp_path).ok().as_deref() == Some(stamp.as_str()) {
            return Ok(());
        }
        println!("Updating bundled `std`");
    } else {
        println!("Placing bundled `std` into {}", deps_dir.display());
    }

    std::fs::create_dir_all(&std_dir)
        .with_context(|| format!("failed to create {}", std_dir.display()))?;
    assets::extract::<Std>(&std_dir)?;
    std::fs::write(&stamp_path, &stamp)
        .with_context(|| format!("failed to write {}", stamp_path.display()))?;

    Ok(())
}

/// エンジンを `.biwa_runtime/` に展開し、必要なら依存をインストールする。
///
/// 展開済みで内容も一致していれば何もしない。
/// `node_modules` は展開の対象外なので、更新しても再インストールは走らない。
pub fn ensure_engine(project: &Project) -> Result<()> {
    let runtime_dir = project.runtime_dir();
    let stamp_path = runtime_dir.join(ENGINE_STAMP_FILE);
    let stamp = assets::stamp::<Engine>();

    let up_to_date = std::fs::read_to_string(&stamp_path).ok().as_deref() == Some(stamp.as_str());

    if !up_to_date {
        if runtime_dir.exists() {
            println!("Updating engine in {}", runtime_dir.display());
        } else {
            println!("Placing engine into {}", runtime_dir.display());
        }
        std::fs::create_dir_all(&runtime_dir)
            .with_context(|| format!("failed to create {}", runtime_dir.display()))?;
        assets::extract::<Engine>(&runtime_dir)?;
        std::fs::write(&stamp_path, &stamp)
            .with_context(|| format!("failed to write {}", stamp_path.display()))?;
    }

    if !runtime_dir.join("node_modules").is_dir() {
        npm_install(&runtime_dir)?;
    }

    Ok(())
}

fn npm_install(runtime_dir: &Path) -> Result<()> {
    println!("Installing engine dependencies (npm install)");

    let status = std::process::Command::new("npm")
        .arg("install")
        .current_dir(runtime_dir)
        .status()
        .context("failed to run `npm`; Biwa requires Node.js (with npm) to be installed")?;

    if !status.success() {
        bail!("`npm install` failed in {}", runtime_dir.display());
    }

    Ok(())
}

/// コンパイル結果をエンジンから見える場所へ移し、エントリポイントのスタブを書く。
///
/// biwac は自パッケージと推移的依存の `.ts` を 1 つのディレクトリに並べて出力する
/// (相互 import が `./<package>.ts` のため)。その関係を保ったまま丸ごと移す。
pub fn sync_generated(project: &Project) -> Result<()> {
    let src_dir = project.generated_typescript_dir();
    let dst_dir = project.runtime_dir().join(GAME_DIR);

    if !src_dir.is_dir() {
        bail!(
            "no compiler output found at {} (did the build succeed?)",
            src_dir.display()
        );
    }

    std::fs::create_dir_all(&dst_dir)
        .with_context(|| format!("failed to create {}", dst_dir.display()))?;

    let mut placed = std::collections::HashSet::new();
    for entry in std::fs::read_dir(&src_dir)
        .with_context(|| format!("failed to read {}", src_dir.display()))?
    {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ts") {
            continue;
        }
        let Some(file_name) = path.file_name() else {
            continue;
        };

        let contents = std::fs::read(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        write_if_changed(&dst_dir.join(file_name), &contents)?;
        placed.insert(file_name.to_os_string());
    }

    if placed.is_empty() {
        bail!("no generated TypeScript found in {}", src_dir.display());
    }

    write_entry_stub(project, &dst_dir)?;
    placed.insert(std::ffi::OsString::from(ENTRY_FILE));

    // 依存から外れたパッケージの `.ts` を残すと、古いコードが混ざり続ける。
    for entry in std::fs::read_dir(&dst_dir)
        .with_context(|| format!("failed to read {}", dst_dir.display()))?
    {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ts") {
            continue;
        }
        let is_stale = path
            .file_name()
            .is_none_or(|name| !placed.contains(name));
        if is_stale {
            std::fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
    }

    Ok(())
}

/// 中身が変わっていなければ書かない。
///
/// 毎回上書きすると、内容が同じでも Vite がリロードを撒いてしまう。
fn write_if_changed(path: &Path, contents: &[u8]) -> Result<()> {
    if std::fs::read(path).is_ok_and(|current| current == contents) {
        return Ok(());
    }
    std::fs::write(path, contents)
        .with_context(|| format!("failed to write {}", path.display()))
}

/// エンジンが呼ぶスタブを生成する。
///
/// 生成物のシンボルはマングルされていてパッケージごとに名前が変わるが、
/// エントリポイントだけは `__biwa_entrypoint` という固定名で export されている。
/// それをさらに固定のファイル名・固定の default export に均し、
/// エンジンがパッケージ名を知らなくても済むようにする。
fn write_entry_stub(project: &Project, dst_dir: &Path) -> Result<()> {
    let path = dst_dir.join(ENTRY_FILE);
    let pkg = &project.name;

    let contents = format!(
        r#"// AUTO-GENERATED by `biwa dev`. Do not edit.
//
// コンパイル結果のエントリポイントを、エンジンが知っている形に均すスタブ。
import {{ {entrypoint} }} from "./{pkg}.ts";
import type {{ BiwaEntrypoint }} from "../engine/game";

export const packageName = "{pkg}";

// 生成コードの `Game` 型はマングル名なので、エンジン側の構造的な型に読み替える。
export default {entrypoint} as unknown as BiwaEntrypoint;
"#,
        entrypoint = ENTRYPOINT_NAME,
        pkg = pkg,
    );

    write_if_changed(&path, contents.as_bytes())
}

/// Vite の開発サーバを起動する。
pub fn spawn_vite(project: &Project, port: Option<u16>) -> Result<std::process::Child> {
    let runtime_dir = project.runtime_dir();
    let vite_bin = runtime_dir.join("node_modules").join(".bin").join("vite");

    if !vite_bin.exists() {
        bail!(
            "vite is not installed at {} (try removing {} and running again)",
            vite_bin.display(),
            runtime_dir.display()
        );
    }

    let mut command = std::process::Command::new(vite_bin);
    command.current_dir(&runtime_dir);
    if let Some(port) = port {
        command.arg("--port").arg(port.to_string()).arg("--strictPort");
    }

    command.spawn().context("failed to start the vite dev server")
}

/// 開発サーバが監視するディレクトリ。
///
/// 依存パッケージは監視しない。ビルド成果物がその中 (`deps/<name>/.biwa_build`)
/// に書かれるため、監視すると再ビルドが自分自身を呼び続ける。
pub fn watch_targets(project: &Project) -> Vec<PathBuf> {
    [project.src_dir()].into_iter().filter(|p| p.is_dir()).collect()
}
