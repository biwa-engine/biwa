use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use biwac_base::Target;
use serde::Deserialize;

use crate::manage::name::Requirement;

/// biwac が読むパッケージメタデータのうち、CLI が必要とする部分だけ。
///
/// 依存の解決やバージョン検査はコンパイラの仕事なので、ここでは名前しか見ない。
#[derive(Debug, Deserialize)]
struct PackageMetadata {
    name: String,
    /// 配布物 (デスクトップアプリ) の版に使う。
    #[serde(default)]
    version: Option<String>,
    /// 要るツールチェイン (`crate::manage::name::Requirement`)。
    #[serde(default)]
    toolchain: Option<Requirement>,
}

/// 開発対象の biwa パッケージ。
#[derive(Debug, Clone)]
pub struct Project {
    /// `biwa-package.json` があるディレクトリ。
    pub root: PathBuf,
    /// パッケージ名。生成物 `<name>.wasm` のファイル名でもある。
    pub name: String,
    /// パッケージの版 (`biwa-package.json` に書かれていれば)。
    pub version: Option<String>,
    /// 要るツールチェイン (`biwa-package.json` の `toolchain`)。
    pub toolchain: Option<Requirement>,
}

impl Project {
    /// 指定パスから上へ辿って `biwa-package.json` を探す。
    pub fn discover(start: &Path) -> Result<Self> {
        match find_metadata(start)? {
            Some((dir, metadata_path)) => Self::load(&dir, &metadata_path),
            None => bail!(
                "`{}` not found in {} or any parent directory",
                biwac_base::METADATA_FILE_NAME,
                start.display()
            ),
        }
    }

    /// この CLI (のツールチェイン) がパッケージの `toolchain` を満たすか確かめる。
    ///
    /// ランチャーは満たすものを選んで起動するので、ここで落ちるのはツールチェインの CLI を
    /// 直接起動したときである。手元でビルドした CLI (名前が `-dirty` / `unknown`) は警告に留める。
    pub fn check_toolchain(&self) -> Result<()> {
        let Some(req) = &self.toolchain else {
            return Ok(());
        };
        let this = crate::toolchain::NAME;
        if req.satisfied_by(this) {
            return Ok(());
        }
        let message = format!(
            "this package requires the Biwa toolchain {req} (`toolchain` in {}), but this CLI is `{this}`",
            biwac_base::METADATA_FILE_NAME
        );
        if this.ends_with("-dirty") || this == "unknown" {
            eprintln!("warning: {message} (continuing because this CLI is a local build)");
            return Ok(());
        }
        bail!(
            "{message}\nrun it through the launcher (`~/.biwa/bin/biwa`), or install a matching toolchain"
        )
    }

    fn load(root: &Path, metadata_path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(metadata_path)
            .with_context(|| format!("failed to read {}", metadata_path.display()))?;
        let metadata: PackageMetadata = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse {}", metadata_path.display()))?;

        Ok(Self {
            root: root.to_path_buf(),
            name: metadata.name,
            version: metadata.version,
            toolchain: metadata.toolchain,
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

    /// `biwa build` の出力先 (`<root>/dist`)。
    pub fn dist_dir(&self) -> PathBuf {
        self.root.join(crate::DIST_DIRECTORY_NAME)
    }

    /// `.biwa` ソースが置かれるディレクトリ。開発サーバはここを監視する。
    pub fn src_dir(&self) -> PathBuf {
        self.root.join("src")
    }

    /// アセットが置かれるディレクトリ (`src/` の兄弟)。
    ///
    /// `.biwa` が書くアセットのパスはここを基準とした相対パスである。
    pub fn assets_dir(&self) -> PathBuf {
        self.root.join(crate::ASSETS_DIRECTORY_NAME)
    }
}

/// `start` から上へ辿って `biwa-package.json` を探す。`(パッケージのディレクトリ, そのファイル)`。
fn find_metadata(start: &Path) -> Result<Option<(PathBuf, PathBuf)>> {
    let start = std::fs::canonicalize(start)
        .map(strip_verbatim_prefix)
        .with_context(|| format!("no such directory: {}", start.display()))?;
    let mut dir = start.as_path();
    loop {
        let metadata_path = dir.join(biwac_base::METADATA_FILE_NAME);
        if metadata_path.is_file() {
            return Ok(Some((dir.to_path_buf(), metadata_path)));
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return Ok(None),
        }
    }
}

/// `start` を含むパッケージの `toolchain` (ランチャーが使う)。パッケージの外なら `None`。
///
/// 要件だけを読む。名前など他の項目が壊れていても、ここでは咎めない (振り分けた先の CLI が叱る)。
pub fn find_requirement(start: &Path) -> Result<Option<Requirement>> {
    #[derive(Deserialize)]
    struct OnlyToolchain {
        #[serde(default)]
        toolchain: Option<Requirement>,
    }
    let Ok(Some((_, path))) = find_metadata(start) else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let parsed: OnlyToolchain = serde_json::from_str(&text)
        .with_context(|| format!("failed to read `toolchain` in {}", path.display()))?;
    Ok(parsed.toolchain)
}

/// Windows の `canonicalize` が付ける `\\?\` を外す。
///
/// このパスは子プロセス (node・Vite・Tauri) の引数にも渡るが、
/// `\\?\C:\...` の形は Node.js のツール群がうまく扱えない。
/// ドライブ文字で始まる普通のパスに戻せるときだけ戻す (UNC などはそのまま)。
#[cfg(windows)]
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

#[cfg(not(windows))]
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    path
}
