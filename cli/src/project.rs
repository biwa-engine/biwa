use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use biwac_base::Target;
use serde::Deserialize;

/// biwac が読むパッケージメタデータのうち、CLI が必要とする部分だけ。
///
/// 依存の解決やバージョン検査はコンパイラの仕事なので、ここでは名前しか見ない。
#[derive(Debug, Deserialize)]
struct PackageMetadata {
    name: String,
}

/// 開発対象の biwa パッケージ。
#[derive(Debug, Clone)]
pub struct Project {
    /// `biwa-package.json` があるディレクトリ。
    pub root: PathBuf,
    /// パッケージ名。生成物 `<name>.ts` のファイル名でもある。
    pub name: String,
}

impl Project {
    /// 指定パスから上へ辿って `biwa-package.json` を探す。
    pub fn discover(start: &Path) -> Result<Self> {
        let start = std::fs::canonicalize(start)
            .with_context(|| format!("no such directory: {}", start.display()))?;

        let mut dir = start.as_path();
        loop {
            let metadata_path = dir.join(biwac_base::METADATA_FILE_NAME);
            if metadata_path.is_file() {
                return Self::load(dir, &metadata_path);
            }
            match dir.parent() {
                Some(parent) => dir = parent,
                None => bail!(
                    "`{}` not found in {} or any parent directory",
                    biwac_base::METADATA_FILE_NAME,
                    start.display()
                ),
            }
        }
    }

    fn load(root: &Path, metadata_path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(metadata_path)
            .with_context(|| format!("failed to read {}", metadata_path.display()))?;
        let metadata: PackageMetadata = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse {}", metadata_path.display()))?;

        Ok(Self {
            root: root.to_path_buf(),
            name: metadata.name,
        })
    }

    /// biwac の出力先 (`<root>/.biwa_build`)。
    pub fn build_dir(&self) -> PathBuf {
        self.root.join(biwac_base::BIWA_BUILD_DIRECTORY_NAME)
    }

    /// 生成物が並ぶディレクトリ。
    ///
    /// `arch` の切り落としでシンボルの集合がターゲットごとに変わるため、
    /// biwac は成果物も中間生成物もターゲットごとに分けて置く。
    pub fn generated_dir(&self, target: Target) -> PathBuf {
        self.build_dir().join(target.build_subdir())
    }

    /// 取得済み依存パッケージが並ぶディレクトリ (`<root>/.biwa_build/deps`)。
    pub fn dependencies_dir(&self) -> PathBuf {
        biwac_base::dependencies_dir(&self.root)
    }

    /// エンジン (Node.js 版) を展開する先。Vite のルートでもある。
    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join(crate::RUNTIME_DIRECTORY_NAME)
    }

    /// `.biwa` ソースが置かれるディレクトリ。開発サーバはここを監視する。
    pub fn src_dir(&self) -> PathBuf {
        self.root.join("src")
    }
}
