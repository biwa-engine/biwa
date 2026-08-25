//! `biwa` コマンド。
//!
//! ノベルゲームの開発に必要なもの (コンパイラ・エンジン・標準ライブラリ) を
//! 1 つにまとめた入口である。利用者に用意させるのは Node.js だけで、
//! コンパイラ biwac はライブラリとして直接呼ぶ (`biwac` コマンドは経由しない)。

mod assets;
mod dev;
mod project;
mod runtime;

use clap::{Parser, Subcommand};

/// エンジンを展開するディレクトリ。Vite のルートでもある。
pub const RUNTIME_DIRECTORY_NAME: &str = ".biwa_runtime";

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
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Dev(args) => dev::run(args),
    };

    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
