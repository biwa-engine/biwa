//! `biwa` バイナリに同梱する資産。
//!
//! ユーザーの環境に用意させるのは Node.js だけである。
//! エンジン (Node.js 版) と標準ライブラリ `std` はこのバイナリが持っていて、
//! 必要になった時点でプロジェクト内に展開する。
//!
//! デバッグビルドでは rust-embed が実行時にディスクから読むため、
//! リポジトリ内のエンジンや std を編集するとそのまま反映される。
//! リリースビルドではバイナリに焼き込まれる。

use std::path::Path;

use anyhow::{Context, Result};
use rust_embed::RustEmbed;

/// エンジン (Node.js 版) 一式。Vite プロジェクトそのもの。
#[derive(RustEmbed)]
#[folder = "../engine/nodejs"]
#[exclude = "node_modules/*"]
#[exclude = "dist/*"]
#[exclude = ".direnv/*"]
#[exclude = "flake.*"]
#[exclude = ".envrc"]
#[exclude = "CLAUDE.md"]
pub struct Engine;

/// 標準ライブラリ `std` のソース。
#[derive(RustEmbed)]
#[folder = "../library/std"]
#[exclude = ".biwa_build/*"]
pub struct Std;

/// 埋め込まれたファイル群の同一性。
///
/// 展開済みのものが古くないかの判定に使う。
/// 内容そのものを混ぜているので、エンジンを 1 文字直せば値が変わる。
pub fn stamp<A: RustEmbed>() -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |bytes: &[u8]| {
        for b in bytes {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };

    let mut names: Vec<String> = A::iter().map(|f| f.to_string()).collect();
    names.sort();
    for name in names {
        mix(name.as_bytes());
        if let Some(file) = A::get(&name) {
            mix(&file.data);
        }
    }

    format!("{hash:016x}")
}

/// 埋め込まれたファイル群を `dst` 以下に書き出す。
///
/// 既存ファイルは上書きするが、`dst` 以下の余計なファイル
/// (`node_modules` など、展開後に生えたもの) は消さない。
pub fn extract<A: RustEmbed>(dst: &Path) -> Result<()> {
    for name in A::iter() {
        let file = A::get(&name).expect("embedded file must exist while iterating");
        let path = dst.join(name.as_ref());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        std::fs::write(&path, &file.data)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(())
}
