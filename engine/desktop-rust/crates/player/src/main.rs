//! Biwa のゲームを動かすネイティブのデスクトップのプレイヤー。
//!
//! 部品 (crate) をつなぐだけで、具体的な外部の crate は直接使わない:
//!
//! - `biwa-player-core`: エンジンの論理 (syscall・Message Window・時計・クリック)
//! - `biwa-player-vm`: game.wasm を wasmtime で動かす
//!
//! ゲームを焼き込まない汎用の実行ファイルで、`game/game.wasm` を読んで動く
//! (`docs/biwa-prebuilt-player.md` の方式)。

mod headless;
mod options;

use options::Options;

fn main() {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };

    if let Err(message) = run(&options) {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}

fn run(options: &Options) -> Result<(), String> {
    let wasm = std::fs::read(&options.wasm)
        .map_err(|e| format!("failed to read {}: {e}", options.wasm.display()))?;
    if options.headless {
        return headless::run(options, wasm);
    }
    Err("only --headless is implemented so far".into())
}
