//! `biwa` コマンド。
//!
//! ノベルゲームの開発に必要なもの (コンパイラ・エンジン・標準ライブラリ) を
//! 1 つにまとめた入口である。利用者に用意させるのは Node.js だけで、
//! コンパイラ biwac はライブラリとして直接呼ぶ (`biwac` コマンドは経由しない)。

mod assets;
mod dev;
mod project;
mod publish;
mod runtime;

use clap::{Parser, Subcommand};

/// エンジンを展開するディレクトリ。Vite のルートでもある。
pub const RUNTIME_DIRECTORY_NAME: &str = ".biwa_runtime";

/// アセットを置くディレクトリ。パッケージ直下、`src/` の兄弟である。
///
/// `.biwa` が書くアセットのパスはこのディレクトリを基準とした相対パスで、
/// エンジンからは同じ名前で公開される (`runtime::link_assets`)。
pub const ASSETS_DIRECTORY_NAME: &str = "assets";

#[derive(Debug, Parser)]
#[command(name = "biwa", version, about = "Biwa novel game engine", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 開発サーバを起動する
    Dev(dev::DevArgs),

    /// パッケージを Biwa Package Hub に公開する
    Publish(publish::PublishArgs),
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Dev(args) => dev::run(args),
        Command::Publish(args) => publish::run(args),
    };

    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
