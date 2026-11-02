//! 起動時の引数。
//!
//! ```text
//! biwa-player [--game-dir <DIR> | --wasm <FILE>] [--assets <DIR>] [--headless] [--trace] [--collector <auto|drc|null>]
//! ```
//!
//! ゲームの場所 (指定が無いとき) は Tauri 版のプレイヤーと同じく、
//! 実行ファイルの横の `game/` (macOS は `<name>.app/Contents/Resources/game/`)。
//! その中の `game.wasm` を読む。
//!
//! アセット (画像など) の置き場所は `--assets`、無ければ `<game>/assets`。
//! `--wasm` で `<project>/.biwa_build/wasm/<package>.wasm` を直接指したときは `<project>/assets` を使う
//! (開発中のプロジェクトをそのまま動かすため)。

use std::path::{Path, PathBuf};

use biwa_player_vm::Collector;

/// `game/` の中の wasm の名前。
const WASM_FILE: &str = "game.wasm";

#[derive(Debug, Clone)]
pub struct Options {
    /// 読む wasm。
    pub wasm: PathBuf,
    /// アセット (`.biwa` が書くパスの基準) のディレクトリ。分からなければ `None` (画像は出ない)。
    pub assets: Option<PathBuf>,
    /// ウィンドウを出さずに動かす (クリック待ちは自動で進める)。
    pub headless: bool,
    /// syscall を 1 つずつログに出す。
    pub trace: bool,
    pub collector: Collector,
}

impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut wasm = None;
        let mut game_dir = None;
        let mut assets = None;
        let mut headless = false;
        let mut trace = false;
        let mut collector = Collector::Auto;

        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
            match arg.as_str() {
                "--wasm" => wasm = Some(PathBuf::from(value("--wasm")?)),
                "--game-dir" => game_dir = Some(PathBuf::from(value("--game-dir")?)),
                "--assets" => assets = Some(PathBuf::from(value("--assets")?)),
                "--headless" => headless = true,
                "--trace" => trace = true,
                "--collector" => {
                    collector = match value("--collector")?.as_str() {
                        "auto" => Collector::Auto,
                        "drc" => Collector::DeferredReferenceCounting,
                        "null" => Collector::Null,
                        other => return Err(format!("unknown collector `{other}` (auto, drc, null)")),
                    }
                }
                "-h" | "--help" => return Err(USAGE.into()),
                other => return Err(format!("unknown argument `{other}`\n{USAGE}")),
            }
        }

        let game_dir = game_dir.or_else(default_game_dir);
        let wasm = match (wasm, &game_dir) {
            (Some(wasm), _) => wasm,
            (None, Some(dir)) => dir.join(WASM_FILE),
            (None, None) => {
                return Err(format!(
                    "no game found next to the player (expected `game/{WASM_FILE}`); pass --game-dir or --wasm\n{USAGE}"
                ));
            }
        };
        let assets = assets
            .or_else(|| game_dir.as_ref().map(|dir| dir.join("assets")))
            .or_else(|| project_assets(&wasm))
            .filter(|dir| dir.is_dir());
        Ok(Self { wasm, assets, headless, trace, collector })
    }
}

const USAGE: &str = "usage: biwa-player [--game-dir <DIR> | --wasm <FILE>] [--assets <DIR>] [--headless] [--trace] [--collector <auto|drc|null>]";

/// `<project>/.biwa_build/wasm/<package>.wasm` なら `<project>/assets`。
fn project_assets(wasm: &Path) -> Option<PathBuf> {
    let build_dir = wasm.parent()?.parent()?;
    (build_dir.file_name()? == ".biwa_build").then(|| build_dir.parent().map(|p| p.join("assets")))?
}

/// 実行ファイルの横の `game/`。無ければ `None`。
fn default_game_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    [
        exe_dir.join("game"),
        // macOS: <name>.app/Contents/MacOS/<exe> から Contents/Resources/game へ。
        exe_dir.join("..").join("Resources").join("game"),
    ]
    .into_iter()
    .find(|dir| has_wasm(dir))
}

fn has_wasm(dir: &Path) -> bool {
    dir.join(WASM_FILE).is_file()
}
