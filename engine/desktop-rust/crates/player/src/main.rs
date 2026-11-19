//! Biwa のゲームを動かすネイティブのデスクトップのプレイヤー。
//!
//! 部品 (crate) をつなぐだけで、具体的な外部の crate は直接使わない:
//!
//! - `biwa-player-core`: エンジンの論理 (syscall・Message Window・時計・クリック)
//! - `biwa-player-vm`: game.wasm を wasmtime で動かす
//! - `biwa-player-text`: 文字の整形とラスタライズ
//! - `biwa-player-image`: 画像の読み込みとデコード
//! - `biwa-player-layout`: UI のレイアウト
//! - `biwa-player-render`: 描画
//! - `biwa-player-platform`: ウィンドウ・入力・イベントループ
//!
//! ゲームを焼き込まない汎用の実行ファイルで、`game/game.wasm` を読んで動く
//! (`docs/biwa-prebuilt-player.md` の方式)。

// リリースビルドの Windows では、ダブルクリックで起動したときにコンソールを開かない。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dev;
mod headless;
mod options;
mod windowed;

use options::Options;

fn main() {
    biwa_player_platform::attach_parent_console();
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };

    if let Err(message) = run(&options) {
        eprintln!("error: {message}");
        write_error_log(&message);
        std::process::exit(1);
    }
}

/// エラーを一時ディレクトリの `biwa-runtime-error.log` にも書く。
///
/// Windows のリリースビルドはコンソールを持たず、ダブルクリックで起動すると標準エラーが見えないため。
fn write_error_log(message: &str) {
    let path = std::env::temp_dir().join("biwa-runtime-error.log");
    let _ = std::fs::write(&path, format!("error: {message}\n"));
}

fn run(options: &Options) -> Result<(), String> {
    let wasm = std::fs::read(&options.wasm)
        .map_err(|e| format!("failed to read {}: {e}", options.wasm.display()))?;
    if options.headless {
        return headless::run(options, wasm);
    }
    windowed::run(options, wasm)
}
