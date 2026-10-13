//! ビルド済みのプレイヤー (`engine/desktop-tauri` の実行ファイル) を探す。
//!
//! プレイヤーはゲームに依らない汎用の実行ファイルで、Biwa の CI が OS ごとに 1 度だけビルドする。
//! `biwa build` は利用者の手元でそれをビルドせず、置いてあるものを使う
//! (`~/memo/biwa-prebuilt-player.md`)。
//!
//! 置き場所 (上から順に探す):
//!   1. `--players-dir <DIR>`
//!   2. 環境変数 `BIWA_PLAYERS_DIR`
//!   3. `~/.biwa/players`
//!
//! その下に OS ごとのディレクトリを切る。名前は CI の成果物と揃えてある。
//!
//! ```text
//! <players>/windows-x86_64/biwa-player.exe
//! <players>/macos-universal/biwa-player
//! <players>/linux-x86_64/biwa-player
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

    /// `dist/` の下のディレクトリ名、配布物のファイル名に使う名前。
    pub fn name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
        }
    }

    /// プレイヤーの置き場所のディレクトリ名 (CI の成果物の名前から `biwa-player-` を除いたもの)。
    pub fn player_id(self) -> &'static str {
        match self {
            Self::Windows => "windows-x86_64",
            // Apple Silicon と Intel の両方で動く universal バイナリ。
            Self::Macos => "macos-universal",
            Self::Linux => "linux-x86_64",
        }
    }

    fn player_file(self) -> &'static str {
        match self {
            Self::Windows => "biwa-player.exe",
            Self::Macos | Self::Linux => "biwa-player",
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

/// `os` のプレイヤーの実行ファイル。
pub fn find(players_dir: &Path, os: DesktopOs) -> Result<PathBuf> {
    let path = players_dir.join(os.player_id()).join(os.player_file());
    if !path.is_file() {
        bail!(
            "the Biwa player for {} is not found at {}\n\
             (place the `biwa-player-{}` build artifact there, or pass --players-dir)",
            os.name(),
            path.display(),
            os.player_id()
        );
    }
    Ok(path)
}
