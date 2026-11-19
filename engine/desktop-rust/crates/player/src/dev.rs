//! 開発モード (`--dev`)。`biwa dev` が起動し、標準入力で指示を送る。
//!
//! 1 行に 1 つの指示:
//!   - `reload`: wasm とアセットを読み直し、ゲームを最初からやり直す (ウィンドウは開いたまま)。
//!
//! 標準入力が閉じたら (`biwa dev` が終わったら) 終わる。

use std::io::BufRead;
use std::sync::mpsc::{self, Receiver};

/// `biwa dev` からの指示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevCommand {
    Reload,
    /// 標準入力が閉じた。
    Quit,
}

/// 標準入力を読むスレッドを立てる。指示が届くたびに `wake` を呼ぶ (イベントループを起こす)。
pub fn listen(wake: impl Fn() + Send + 'static) -> Receiver<DevCommand> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("biwa-dev".into()).spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let command = match line.trim() {
                "" => continue,
                "reload" => DevCommand::Reload,
                other => {
                    eprintln!("[biwa] unknown dev command `{other}`");
                    continue;
                }
            };
            if tx.send(command).is_err() {
                return;
            }
            wake();
        }
        let _ = tx.send(DevCommand::Quit);
        wake();
    });
    if let Err(e) = spawned {
        eprintln!("[biwa] failed to start the dev command thread: {e}");
    }
    rx
}
