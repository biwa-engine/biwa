//! ビルド済みのプレイヤー (ランタイム) を探す。
//!
//! プレイヤーは 2 種類ある。
//!   - ネイティブ (`engine/desktop-rust`): wasmtime を同梱した Rust 製。`--platform windows|macos|linux`
//!   - WebView (`engine/desktop-tauri`): OS の WebView で動く Tauri 製。`--platform windows-webview|...`
//!
//! プレイヤーはゲームに依らない汎用の実行ファイルで、Biwa の CI が OS ごとに 1 度だけビルドする。
//! `biwa build` は利用者の手元でそれをビルドせず、置いてあるものを使う
//! (`docs/biwa-prebuilt-player.md`)。
//!
//! 置き場所 (上から順に探す):
//!   1. `--players-dir <DIR>`
//!   2. 環境変数 `BIWA_PLAYERS_DIR`
//!   3. `~/.biwa/players`
//!
//! その下に OS ごとのディレクトリを切る。名前は CI の成果物と揃えてある。
//!
//! ```text
//! <players>/windows-x86_64/biwa-player.exe            ネイティブ
//! <players>/macos-universal/biwa-player
//! <players>/linux-x86_64/biwa-player
//! <players>/windows-webview-x86_64/biwa-player.exe    WebView
//! <players>/macos-webview-universal/biwa-player
//! <players>/linux-webview-x86_64/biwa-player
//! ```
//!
//! 将来は `biwa` に同梱するか、版の合うものを初回にダウンロードする。

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

/// プレイヤーの種類。
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

    /// プレイヤーの置き場所のディレクトリ名 (CI の成果物の名前から `biwa-player-` を除いたもの)。
    pub fn player_id(self) -> String {
        format!("{}-{}", self.name(), self.os.arch())
    }

    fn player_file(self) -> &'static str {
        match self.os {
            DesktopOs::Windows => "biwa-player.exe",
            DesktopOs::Macos | DesktopOs::Linux => "biwa-player",
        }
    }
}

/// プレイヤーの置き場所を決める。
pub fn players_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = explicit {
        return Ok(dir.to_path_buf());
    }
    if let Some(dir) = std::env::var_os("BIWA_PLAYERS_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|v| !v.is_empty());
    match home {
        Some(home) => Ok(PathBuf::from(home).join(".biwa").join("players")),
        None => bail!("cannot find the home directory; pass --players-dir"),
    }
}

/// `target` のプレイヤーの実行ファイル。
pub fn find(players_dir: &Path, target: DesktopTarget) -> Result<PathBuf> {
    let path = players_dir
        .join(target.player_id())
        .join(target.player_file());
    if !path.is_file() {
        bail!(
            "the Biwa player for {} is not found at {}\n\
             (place the `biwa-player-{}` build artifact there, or pass --players-dir)",
            target.name(),
            path.display(),
            target.player_id()
        );
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_player_ids() {
        let native = DesktopTarget::new(DesktopOs::Macos, Runtime::Native);
        let webview = DesktopTarget::new(DesktopOs::Windows, Runtime::Webview);
        assert_eq!(
            (native.name(), native.player_id()),
            ("macos".into(), "macos-universal".into())
        );
        assert_eq!(
            (webview.name(), webview.player_id()),
            ("windows-webview".into(), "windows-webview-x86_64".into())
        );
    }
}
