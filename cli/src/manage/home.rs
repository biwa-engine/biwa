//! `~/.biwa` (`BIWA_HOME`) の中身: インストール済みのツールチェインと設定。
//!
//! ```text
//! <home>/bin/biwa[.exe]                      ← ランチャー
//! <home>/toolchains/<name>/bin/biwa[.exe]    ← ツールチェインの実体
//! <home>/toolchains/<name>/runtime/<id>/     ← runtime
//! <home>/settings.json                       ← { "default": "<name>" }
//! ```

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::name::{Requirement, ToolchainName, Version, validate};
use crate::toolchain::biwa_home;

/// 実行ファイルの名前。
pub const EXE: &str = if cfg!(windows) { "biwa.exe" } else { "biwa" };

/// `~/.biwa` の場所と、その中の決まった場所。
#[derive(Debug, Clone)]
pub struct Home {
    pub root: PathBuf,
}

/// `settings.json`。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    /// 既定のツールチェイン。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

impl Home {
    pub fn locate() -> Result<Self> {
        Ok(Self { root: biwa_home()? })
    }

    /// ランチャー (`<home>/bin/biwa`)。
    pub fn launcher(&self) -> PathBuf {
        self.root.join("bin").join(EXE)
    }

    pub fn toolchains_dir(&self) -> PathBuf {
        self.root.join("toolchains")
    }

    pub fn toolchain_dir(&self, name: &str) -> PathBuf {
        self.toolchains_dir().join(name)
    }

    /// ツールチェインの実体の `biwa`。
    pub fn toolchain_exe(&self, name: &str) -> PathBuf {
        self.toolchain_dir(name).join("bin").join(EXE)
    }

    /// ダウンロードや展開の途中のものを置く場所。`toolchains/` と同じファイルシステムにあるので、
    /// 出来上がったものを rename で置ける。
    pub fn tmp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }

    fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn settings(&self) -> Result<Settings> {
        let path = self.settings_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("failed to parse {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(e).with_context(|| format!("failed to read {}", path.display())),
        }
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let path = self.settings_path();
        std::fs::create_dir_all(&self.root)?;
        let text = serde_json::to_string_pretty(settings)? + "\n";
        // 途中で落ちても壊れたファイルが残らないよう、書いてから置き換える。
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("failed to write {}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    pub fn default_toolchain(&self) -> Result<Option<String>> {
        Ok(self.settings()?.default)
    }

    pub fn set_default(&self, name: &str) -> Result<()> {
        let mut settings = self.settings()?;
        settings.default = Some(name.to_string());
        self.save_settings(&settings)
    }

    /// `toolchains/` にあるツールチェインの名前。版は新しい順、それ以外は名前順で後ろに並べる。
    pub fn installed(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.toolchains_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| validate(n).is_ok())
            .collect();
        sort_newest_first(&mut names);
        names
    }

    pub fn is_installed(&self, name: &str) -> bool {
        self.toolchain_dir(name).is_dir()
    }

    /// ランチャーが振り分ける先のツールチェインを決める。
    ///
    /// 1. `explicit` (`--toolchain`、無ければ環境変数 `BIWA_TOOLCHAIN`)
    /// 2. パッケージの要件 (`biwa-package.json` の `toolchain`)
    ///    - 版: 既定が満たせば既定、でなければインストール済みで満たす最新のもの
    ///    - ハッシュなど: 同じ名前のもの
    /// 3. 既定
    ///
    /// 満たすものが無ければエラーにする (自動のインストールはまだしない)。
    pub fn select(
        &self,
        explicit: Option<&str>,
        requirement: Option<&Requirement>,
    ) -> Result<String> {
        if let Some(name) = explicit {
            validate(name)?;
            if !self.is_installed(name) {
                bail!("{}", self.not_installed(name));
            }
            // 指定しても要件は外せない (コンパイラはツールチェインの一部なので、満たさないものでは作れない)。
            if let Some(req) = requirement
                && !req.satisfied_by(name)
            {
                bail!(
                    "the toolchain `{name}` does not satisfy this package's requirement {req} (`toolchain` in {})",
                    biwac_base::METADATA_FILE_NAME
                );
            }
            return Ok(name.to_string());
        }

        let default = self.default_toolchain()?;
        if let Some(req) = requirement {
            if let Some(d) = default
                .as_deref()
                .filter(|d| req.satisfied_by(d) && self.is_installed(d))
            {
                return Ok(d.to_string());
            }
            // installed() は版の新しい順なので、最初に満たしたものが最新である。
            if let Some(found) = self.installed().into_iter().find(|n| req.satisfied_by(n)) {
                return Ok(found);
            }
            let how = match &req.min {
                ToolchainName::Version(v) => {
                    format!("`biwa toolchain install {v}` (or a newer version)")
                }
                ToolchainName::Other(s) => format!("`biwa toolchain install {s}`"),
            };
            bail!(
                "this package requires the Biwa toolchain {req} (`toolchain` in {}), but none is installed\n\
                 install it with {how}\ninstalled toolchains: {}",
                biwac_base::METADATA_FILE_NAME,
                self.describe_installed(),
            );
        }

        match default {
            Some(d) if self.is_installed(&d) => Ok(d),
            Some(d) => bail!("{}", self.not_installed(&d)),
            None => bail!(
                "no default Biwa toolchain is set\n\
                 install one with `biwa toolchain install`, or pick one with `biwa toolchain default <NAME>`\n\
                 installed toolchains: {}",
                self.describe_installed()
            ),
        }
    }

    pub fn not_installed(&self, name: &str) -> String {
        format!(
            "the Biwa toolchain `{name}` is not installed (install it with `biwa toolchain install {name}`)\n\
             installed toolchains: {}",
            self.describe_installed()
        )
    }

    pub fn describe_installed(&self) -> String {
        let names = self.installed();
        if names.is_empty() {
            "(none)".to_string()
        } else {
            names.join(", ")
        }
    }
}

/// 版は新しい順に前へ、それ以外は名前順で後ろへ。
pub fn sort_newest_first(names: &mut [String]) {
    names.sort_by(|a, b| {
        let va = a.parse::<Version>().ok();
        let vb = b.parse::<Version>().ok();
        match (va, vb) {
            (Some(x), Some(y)) => y.cmp(&x),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.cmp(b),
        }
    });
}

/// この実行ファイルがランチャー (`<home>/bin/biwa`) として置かれているか。
pub fn running_as_launcher(home: &Home) -> bool {
    let (Ok(me), Ok(launcher)) = (
        std::env::current_exe().and_then(std::fs::canonicalize),
        std::fs::canonicalize(home.launcher()),
    ) else {
        return false;
    };
    me == launcher
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home_with(names: &[&str], default: Option<&str>) -> (tempdir::TempDir, Home) {
        let dir = tempdir::TempDir::new();
        let home = Home {
            root: dir.path().to_path_buf(),
        };
        for n in names {
            std::fs::create_dir_all(home.toolchain_exe(n).parent().unwrap()).unwrap();
        }
        if let Some(d) = default {
            home.set_default(d).unwrap();
        }
        (dir, home)
    }

    fn req(s: &str) -> Requirement {
        serde_json::from_str(&format!("\"{s}\"")).unwrap()
    }

    #[test]
    fn lists_versions_first() {
        let (_d, home) = home_with(&["aaaa", "0.2.0", "0.10.0", "local"], None);
        assert_eq!(home.installed(), ["0.10.0", "0.2.0", "aaaa", "local"]);
    }

    #[test]
    fn selects_by_requirement_and_default() {
        let (_d, home) = home_with(&["0.1.0", "0.3.0", "0.2.0", "3f2a9c1b7d4e"], Some("0.2.0"));
        // 既定が満たせば既定。
        assert_eq!(home.select(None, Some(&req("0.1.0"))).unwrap(), "0.2.0");
        // 既定が満たさなければ、満たす最新。
        assert_eq!(home.select(None, Some(&req("0.2.1"))).unwrap(), "0.3.0");
        assert!(home.select(None, Some(&req("0.4.0"))).is_err());
        // ハッシュはそのもの。
        assert_eq!(
            home.select(None, Some(&req("3f2a9c1b7d4e"))).unwrap(),
            "3f2a9c1b7d4e"
        );
        assert!(home.select(None, Some(&req("bbbbbbbbbbbb"))).is_err());
        // 指定が最優先。ただし要件は満たさなければならない。
        assert_eq!(
            home.select(Some("0.3.0"), Some(&req("0.1.0"))).unwrap(),
            "0.3.0"
        );
        assert!(home.select(Some("0.1.0"), Some(&req("0.3.0"))).is_err());
        assert!(home.select(Some("9.9.9"), None).is_err());
        // 要件が無ければ既定。
        assert_eq!(home.select(None, None).unwrap(), "0.2.0");
    }

    #[test]
    fn needs_a_default_without_a_requirement() {
        let (_d, home) = home_with(&["0.1.0"], None);
        assert!(
            home.select(None, None)
                .unwrap_err()
                .to_string()
                .contains("no default")
        );
    }

    /// テスト用の一時ディレクトリ (依存を足さないための最小限のもの)。
    mod tempdir {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU32, Ordering};

        pub struct TempDir(PathBuf);

        impl TempDir {
            pub fn new() -> Self {
                static N: AtomicU32 = AtomicU32::new(0);
                let p = std::env::temp_dir().join(format!(
                    "biwa-home-test-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::Relaxed)
                ));
                let _ = std::fs::remove_dir_all(&p);
                std::fs::create_dir_all(&p).unwrap();
                Self(p)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
