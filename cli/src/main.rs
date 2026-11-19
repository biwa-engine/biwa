//! `biwa` コマンド。
//!
//! ノベルゲームの開発に必要なもの (コンパイラ・エンジン・標準ライブラリ) を
//! 1 つにまとめた入口である。利用者に用意させるのは Node.js だけで、
//! コンパイラ biwac はライブラリとして直接呼ぶ (`biwac` コマンドは経由しない)。

mod assets;
mod build;
mod desktop;
mod dev;
mod files;
mod manage;
mod project;
mod publish;
mod serve;
mod toolchain;

use clap::{Parser, Subcommand};

/// `biwa build` の出力先。
pub const DIST_DIRECTORY_NAME: &str = "dist";

/// アセットを置くディレクトリ。パッケージ直下、`src/` の兄弟である。
///
/// `.biwa` が書くアセットのパスはこのディレクトリを基準とした相対パスで、
/// runtime からも同じ名前で見える (配布物の `assets/`、`biwa dev` の `/assets/`)。
pub const ASSETS_DIRECTORY_NAME: &str = "assets";

#[derive(Debug, Parser)]
#[command(name = "biwa", version = toolchain::VERSION, about = "Biwa novel game engine", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 開発サーバを起動する
    Dev(dev::DevArgs),

    /// 配布物をビルドする (Web の静的ファイル、またはデスクトップアプリ)
    Build(build::BuildArgs),

    /// パッケージを Biwa Package Hub に公開する
    Publish(publish::PublishArgs),

    /// ツールチェインを入れる・選ぶ・消す
    Toolchain {
        #[command(subcommand)]
        command: manage::ToolchainCommand,
    },

    /// 既定のツールチェインを最新にする (ランチャーも)
    Update,

    /// ランチャー (`~/.biwa/bin/biwa`) 自身の管理
    #[command(name = "self")]
    SelfManage {
        #[command(subcommand)]
        command: manage::SelfCommand,
    },
}

fn main() {
    // `~/.biwa/bin/biwa` (ランチャー) なら、管理以外のコマンドはツールチェインの CLI に渡す。
    if let Some(result) = manage::forward_if_launcher() {
        match result {
            Ok(code) => std::process::exit(code),
            Err(e) => {
                eprintln!("error: {e:#}");
                std::process::exit(1);
            }
        }
    }

    let cli = Cli::parse();

    let result = match cli.command {
        Command::Dev(args) => dev::run(args),
        Command::Build(args) => build::run(args),
        Command::Publish(args) => publish::run(args),
        Command::Toolchain { command } => manage::run_toolchain(command),
        Command::Update => manage::run_update(),
        Command::SelfManage { command } => manage::run_self(command),
    };

    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
