//! プロジェクトに置くファイルの扱い (同梱の std の展開・アセットのコピーなど)。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::assets::{self, Std};
use crate::project::Project;

/// std を展開したときの同一性を記録するファイル。
///
/// このファイルが無い `deps/std` は「利用者が自分で用意したもの」とみなし、
/// CLI は一切触らない (リポジトリのシンボリックリンクなど)。
const STD_STAMP_FILE: &str = ".biwa-std-stamp";

/// 依存パッケージとして `std` を用意する。
///
/// biwac は依存が `<root>/.biwa_build/deps/<name>/` に取得済みであることを前提とする。
/// 取得の実装はまだ無いので、同梱している std をここに置くのが CLI の仕事になる。
pub fn ensure_std(project: &Project) -> Result<()> {
    ensure_std_in(&project.dependencies_dir().join("std"))
}

fn ensure_std_in(std_dir: &Path) -> Result<()> {
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
        // 上書きで展開するだけだと、新しい std で消えたファイルが残る
        // (残った `.biwa` は「宣言されていないモジュール」としてコンパイルを止める)。
        // 印のあるディレクトリは CLI が置いたものなので、丸ごと消してから置き直してよい。
        std::fs::remove_dir_all(std_dir)
            .with_context(|| format!("failed to remove {}", std_dir.display()))?;
    } else {
        println!("Placing bundled `std` into {}", std_dir.display());
    }

    std::fs::create_dir_all(std_dir)
        .with_context(|| format!("failed to create {}", std_dir.display()))?;
    assets::extract::<Std>(std_dir)?;
    std::fs::write(&stamp_path, &stamp)
        .with_context(|| format!("failed to write {}", stamp_path.display()))?;

    Ok(())
}

/// 生成物の中身から決まる短い値。FNV-1a。
///
/// wasm の URL は再ビルドしても変わらないので、これを付けて
/// ブラウザが古いモジュールを掴んだままになるのを防ぐ (`biwa-game.json` の `buildId`)。
pub fn content_id(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// パッケージの `assets/` を `<out_dir>/assets` へコピーする。
///
/// 配布物は自己完結していなければならないので、リンクにはしない。出力側を消してからコピーし直す。
pub fn copy_assets(project: &Project, out_dir: &Path) -> Result<()> {
    let dst = out_dir.join(crate::ASSETS_DIRECTORY_NAME);

    match std::fs::symlink_metadata(&dst) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&dst),
        Ok(_) => std::fs::remove_file(&dst),
        Err(_) => Ok(()),
    }
    .with_context(|| format!("failed to remove {}", dst.display()))?;

    let src = project.assets_dir();
    if src.is_dir() {
        copy_dir(&src, &dst)?;
    }

    Ok(())
}

/// ディレクトリを再帰的にコピーする。リンクは辿る。
pub fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("failed to create {}", dst.display()))?;

    for entry in
        std::fs::read_dir(src).with_context(|| format!("failed to read {}", src.display()))?
    {
        let path = entry?.path();
        let Some(name) = path.file_name() else {
            continue;
        };
        let target = dst.join(name);

        if path.is_dir() {
            copy_dir(&path, &target)?;
        } else {
            std::fs::copy(&path, &target).with_context(|| {
                format!("failed to copy {} to {}", path.display(), target.display())
            })?;
        }
    }

    Ok(())
}

/// `biwa dev` が監視するソースのディレクトリ。
///
/// 依存パッケージは監視しない。ビルド成果物がその中 (`deps/<name>/.biwa_build`)
/// に書かれるため、監視すると再ビルドが自分自身を呼び続ける。
pub fn watch_targets(project: &Project) -> Vec<PathBuf> {
    [project.src_dir()]
        .into_iter()
        .filter(|p| p.is_dir())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("biwa-files-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn updating_std_removes_files_the_new_std_does_not_have() {
        let std_dir = temp_dir("update").join("std");
        ensure_std_in(&std_dir).unwrap();
        // 古い std の名残と、古い印。
        let stale = std_dir.join("src").join("stale_module.biwa");
        std::fs::write(&stale, "").unwrap();
        std::fs::write(std_dir.join(STD_STAMP_FILE), "old").unwrap();

        ensure_std_in(&std_dir).unwrap();
        assert!(!stale.exists());
        assert!(std_dir.join("src").join("lib.biwa").is_file());
        assert_eq!(
            std::fs::read_to_string(std_dir.join(STD_STAMP_FILE)).unwrap(),
            assets::stamp::<Std>()
        );
    }

    #[test]
    fn std_placed_by_the_user_is_left_alone() {
        let std_dir = temp_dir("user").join("std");
        std::fs::create_dir_all(&std_dir).unwrap();
        let own = std_dir.join("mine.biwa");
        std::fs::write(&own, "").unwrap();

        ensure_std_in(&std_dir).unwrap();
        assert!(own.exists());
        assert!(!std_dir.join(STD_STAMP_FILE).exists());
    }
}
