//! ツールチェインと、その中のビルド済みの runtime を探す。
//!
//! ツールチェインは「この CLI (コンパイラ・std・エンジンの JS を含む) + 全ターゲットの runtime」のまとまりで、
//! バージョンの単位である。コンパイラが出す wasm と runtime の間には約束 (syscall の番号など) があるので、
//! runtime は CLI と同じツールチェインのものを使う。バージョン番号がまだ無いので、名前はビルドしたコミット
//! ([`COMMIT`]、`build.rs` が焼き込む)。
//!
//! runtime は 2 種類ある。
//!   - ネイティブ (`engine/desktop-rust`): wasmtime を同梱した Rust 製。`--platform windows|macos|linux`
//!   - WebView (`engine/desktop-tauri`): OS の WebView で動く Tauri 製。`--platform windows-webview|...`
//!
//! runtime はゲームに依らない汎用の実行ファイルで、Biwa の CI がビルドする (`.github/workflows/release.yml`)。
//! `biwa build` は利用者の手元でそれをビルドせず、置いてあるものを使う (`docs/biwa-prebuilt-player.md`)。
//!
//! 置き場所 (`<home>` は環境変数 `BIWA_HOME`、無ければ `~/.biwa`):
//!
//! ```text
//! <home>/bin/biwa[.exe]                                     ← この CLI
//! <home>/toolchains/<toolchain>/runtime/<runtime-id>/biwa-runtime[.exe]
//! ```
//!
//! `<runtime-id>` は `native-windows-x86_64` / `native-macos-universal` / `native-linux-x86_64` /
//! `webview-windows-x86_64` / `webview-macos-universal` / `webview-linux-x86_64`。
//! CI の成果物 `biwa-runtime-<runtime-id>` と揃えてある。
//!
//! 使うツールチェインは `--toolchain <NAME>`、環境変数 `BIWA_TOOLCHAIN`、この CLI のコミットの順に決める
//! (前の 2 つは実験用。別のコミットの runtime と組み合わせると動かないことがある)。

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// デスクトップの配布先の OS。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopOs {
    Windows,
    Macos,
    Linux,
}

impl DesktopOs {
    /// `biwa` が動いている OS。
    pub fn host() -> Result<Self> {
        match std::env::consts::OS {
            "windows" => Ok(Self::Windows),
            "macos" => Ok(Self::Macos),
            "linux" => Ok(Self::Linux),
            other => bail!(
                "desktop apps cannot be built for this OS (`{other}`); pick one with --platform"
            ),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
        }
    }

    /// CPU の種類。macOS は Apple Silicon と Intel の両方で動く universal バイナリ。
    fn arch(self) -> &'static str {
        match self {
            Self::Windows | Self::Linux => "x86_64",
            Self::Macos => "universal",
        }
    }
}

/// runtime の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    /// Rust 製 (`engine/desktop-rust`)。ゲームのデータは `game.wasm` と `assets/`。
    Native,
    /// Tauri 製 (`engine/desktop-tauri`)。ゲームのデータは Web の静的ビルド。
    Webview,
}

/// 配布物の行き先 (OS とプレイヤーの種類の組)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopTarget {
    pub os: DesktopOs,
    pub runtime: Runtime,
}

impl DesktopTarget {
    pub const fn new(os: DesktopOs, runtime: Runtime) -> Self {
        Self { os, runtime }
    }

    pub fn os(self) -> DesktopOs {
        self.os
    }

    /// `dist/` の下のディレクトリ名、`--platform` の値 (`windows` / `windows-webview` ...)。
    pub fn name(self) -> String {
        match self.runtime {
            Runtime::Native => self.os.name().to_string(),
            Runtime::Webview => format!("{}-webview", self.os.name()),
        }
    }

    /// 配布物のファイル名に付ける名前 (`linux-x86_64` / `linux-webview-x86_64` ...)。
    pub fn label(self) -> String {
        format!("{}-{}", self.name(), self.os.arch())
    }

    /// runtime の置き場所のディレクトリ名 (CI の成果物の名前から `biwa-runtime-` を除いたもの)。
    pub fn runtime_id(self) -> String {
        let kind = match self.runtime {
            Runtime::Native => "native",
            Runtime::Webview => "webview",
        };
        format!("{kind}-{}-{}", self.os.name(), self.os.arch())
    }

    fn runtime_file(self) -> &'static str {
        match self.os {
            DesktopOs::Windows => "biwa-runtime.exe",
            DesktopOs::Macos | DesktopOs::Linux => "biwa-runtime",
        }
    }
}

/// この CLI をビルドしたコミット (`build.rs`)。既定のツールチェインの名前。
pub const COMMIT: &str = env!("BIWA_COMMIT");

/// `biwa --version` に出す文字列。
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("BIWA_COMMIT"), ")");

/// Biwa の置き場所 (環境変数 `BIWA_HOME`、無ければ `~/.biwa`)。
pub fn biwa_home() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("BIWA_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|v| !v.is_empty());
    match home {
        Some(home) => Ok(PathBuf::from(home).join(".biwa")),
        None => bail!("cannot find the home directory; set BIWA_HOME"),
    }
}

/// 使うツールチェイン。
#[derive(Debug, Clone)]
pub struct Toolchain {
    pub name: String,
    dir: PathBuf,
}

impl Toolchain {
    /// `--toolchain` (`explicit`)、環境変数 `BIWA_TOOLCHAIN`、この CLI のコミットの順に決める。
    pub fn select(explicit: Option<&str>) -> Result<Self> {
        let name = explicit
            .map(str::to_string)
            .or_else(|| {
                std::env::var("BIWA_TOOLCHAIN")
                    .ok()
                    .filter(|v| !v.is_empty())
            })
            .unwrap_or_else(|| COMMIT.to_string());
        if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
            bail!("invalid toolchain name `{name}`");
        }
        let dir = biwa_home()?.join("toolchains").join(&name);
        Ok(Self { name, dir })
    }

    /// `target` の runtime の実行ファイル。
    pub fn runtime(&self, target: DesktopTarget) -> Result<PathBuf> {
        let path = self
            .dir
            .join("runtime")
            .join(target.runtime_id())
            .join(target.runtime_file());
        if path.is_file() {
            return Ok(path);
        }
        let installed = self.installed();
        let hint = if self.name == COMMIT && self.name.ends_with("-dirty") {
            "\nThis CLI was built from uncommitted changes, so no released toolchain matches it; \
             pass --toolchain <NAME> (or set BIWA_TOOLCHAIN) to use one of the installed toolchains."
        } else {
            ""
        };
        bail!(
            "the Biwa runtime for {} is not found at {}\n\
             (extract a Biwa release archive into {}, or place the `biwa-runtime-{}` build artifact there)\n\
             installed toolchains: {}{hint}",
            target.name(),
            path.display(),
            self.home_display(),
            target.runtime_id(),
            if installed.is_empty() {
                "(none)".to_string()
            } else {
                installed.join(", ")
            },
        )
    }

    /// `<home>/toolchains/` にあるツールチェインの名前。
    fn installed(&self) -> Vec<String> {
        let Some(parent) = self.dir.parent() else {
            return Vec::new();
        };
        let mut names: Vec<String> = std::fs::read_dir(parent)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn home_display(&self) -> String {
        self.dir
            .parent()
            .and_then(Path::parent)
            .map_or_else(|| "~/.biwa".to_string(), |p| p.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_runtime_ids() {
        let native = DesktopTarget::new(DesktopOs::Macos, Runtime::Native);
        let webview = DesktopTarget::new(DesktopOs::Windows, Runtime::Webview);
        assert_eq!(native.name(), "macos");
        assert_eq!(native.label(), "macos-universal");
        assert_eq!(native.runtime_id(), "native-macos-universal");
        assert_eq!(webview.name(), "windows-webview");
        assert_eq!(webview.label(), "windows-webview-x86_64");
        assert_eq!(webview.runtime_id(), "webview-windows-x86_64");
    }
}
