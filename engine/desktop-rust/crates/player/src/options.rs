//! 起動時の引数。
//!
//! ```text
//! biwa-runtime [--game-dir <DIR> | --wasm <FILE>] [--assets <DIR>] [--headless] [--trace] [--collector <auto|drc|null>]
//! ```
//!
//! ゲームの場所 (指定が無いとき) は Tauri 版のプレイヤーと同じく、
//! 実行ファイルの横の `game/` (macOS は `<name>.app/Contents/Resources/game/`)。
//! その中の `game.wasm` を読む。
//!
//! アセット (画像など) の置き場所は `--assets`、無ければ `<game>/assets`。
//! `--wasm` で `<project>/.biwa_build/wasm/<package>.wasm` を直接指したときは `<project>/assets` を使う
//! (開発中のプロジェクトをそのまま動かすため)。
//!
//! ウィンドウのタイトルは `<game>/biwa-game.json` の `title` (`biwa build` が書く)。
//! ゲームの画面の大きさ (論理ピクセル。縦横比もこれで決まる) は同じファイルの `width` / `height`、
//! 無ければ 1280x720 (16:9)。

use std::path::{Path, PathBuf};

use biwa_player_vm::Collector;

/// `game/` の中の wasm の名前。
const WASM_FILE: &str = "game.wasm";
/// `game/` の中のゲームの情報 (`cli/src/build.rs` の `write_game_info`)。
const GAME_INFO_FILE: &str = "biwa-game.json";
/// ゲームの情報が無いときのウィンドウのタイトル。
const DEFAULT_TITLE: &str = "Biwa";
/// ゲームの情報が無いときの画面の大きさ (論理ピクセル)。TypeScript 版のプレイヤーの既定のウィンドウと同じ。
const DEFAULT_SCREEN: (u32, u32) = (1280, 720);

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
    /// ウィンドウのタイトル。
    pub title: String,
    /// ゲームの画面の大きさ (論理ピクセル)。
    pub screen: (u32, u32),
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
        let info = game_dir
            .as_ref()
            .and_then(|dir| std::fs::read_to_string(dir.join(GAME_INFO_FILE)).ok())
            .unwrap_or_default();
        let title = json_field(&info, "title")
            .and_then(JsonValue::into_string)
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| DEFAULT_TITLE.into());
        let dimension = |key| {
            json_field(&info, key).and_then(JsonValue::into_number).filter(|n| (1.0..=16384.0).contains(n)).map(|n| n as u32)
        };
        let screen = match (dimension("width"), dimension("height")) {
            (Some(width), Some(height)) => (width, height),
            _ => DEFAULT_SCREEN,
        };
        Ok(Self { wasm, assets, headless, trace, collector, title, screen })
    }
}

const USAGE: &str = "usage: biwa-runtime [--game-dir <DIR> | --wasm <FILE>] [--assets <DIR>] [--headless] [--trace] [--collector <auto|drc|null>]";

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

/// JSON の値 (文字列か、それ以外をそのままの文字列で)。
#[derive(Debug, Clone, PartialEq)]
enum JsonValue {
    String(String),
    Other(String),
}

impl JsonValue {
    fn into_string(self) -> Option<String> {
        match self {
            Self::String(s) => Some(s),
            Self::Other(_) => None,
        }
    }

    fn into_number(self) -> Option<f64> {
        match self {
            Self::Other(s) => s.parse().ok(),
            Self::String(_) => None,
        }
    }
}

/// 平らな JSON のオブジェクトから値を 1 つ取り出す。
///
/// `biwa-game.json` は `{"name": "...", "title": "...", "version": "...", "width": 1280, "height": 720}` だけなので、
/// JSON の crate は入れずにここで読む (入れ子や配列は扱わない)。
fn json_field(json: &str, key: &str) -> Option<JsonValue> {
    let mut chars = json.trim().strip_prefix('{')?.chars().peekable();
    loop {
        skip_whitespace(&mut chars);
        let name = json_string(&mut chars)?;
        skip_whitespace(&mut chars);
        if chars.next()? != ':' {
            return None;
        }
        skip_whitespace(&mut chars);
        let value = if chars.peek() == Some(&'"') {
            JsonValue::String(json_string(&mut chars)?)
        } else {
            // 文字列でない値 (数・真偽値など) は区切りまでをそのまま取る。
            let mut raw = String::new();
            while let Some(c) = chars.next_if(|&c| c != ',' && c != '}') {
                raw.push(c);
            }
            JsonValue::Other(raw.trim().to_string())
        };
        if name == key {
            return Some(value);
        }
        skip_whitespace(&mut chars);
        match chars.next()? {
            ',' => continue,
            _ => return None,
        }
    }
}

fn skip_whitespace(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while chars.peek().is_some_and(|c| c.is_whitespace()) {
        chars.next();
    }
}

fn json_string(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<String> {
    if chars.next()? != '"' {
        return None;
    }
    let mut out = String::new();
    loop {
        match chars.next()? {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'u' => {
                    let high = hex_unit(chars)?;
                    let code = if (0xD800..0xDC00).contains(&high) {
                        // サロゲートペア。
                        if chars.next()? != '\\' || chars.next()? != 'u' {
                            return None;
                        }
                        let low = hex_unit(chars)?;
                        0x10000 + ((high - 0xD800) << 10) + (low.checked_sub(0xDC00)? & 0x3FF)
                    } else {
                        high
                    };
                    out.push(char::from_u32(code)?);
                }
                c => out.push(c),
            },
            c => out.push(c),
        }
    }
}

/// `\u` の後ろの 16 進 4 桁。
fn hex_unit(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<u32> {
    let hex: String = (0..4).map(|_| chars.next()).collect::<Option<_>>()?;
    u32::from_str_radix(&hex, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::{JsonValue, json_field};

    fn json_string_field(json: &str, key: &str) -> Option<String> {
        json_field(json, key).and_then(JsonValue::into_string)
    }

    #[test]
    fn reads_numbers() {
        let json = "{\"title\": \"a\", \"width\": 1920 , \"height\":1080}";
        assert_eq!(json_field(json, "width").and_then(JsonValue::into_number), Some(1920.0));
        assert_eq!(json_field(json, "height").and_then(JsonValue::into_number), Some(1080.0));
        assert_eq!(json_field(json, "title").and_then(JsonValue::into_number), None);
    }

    #[test]
    fn reads_the_title() {
        let json = "{\n  \"name\": \"test1\",\n  \"n\": 1,\n  \"title\": \"\\u3086\\u3081 \\\"x\\\" \\ud83c\\udf38\",\n  \"version\": \"0.1.0\"\n}";
        assert_eq!(json_string_field(json, "title").as_deref(), Some("ゆめ \"x\" 🌸"));
        assert_eq!(json_string_field(json, "name").as_deref(), Some("test1"));
        assert_eq!(json_string_field(json, "missing"), None);
    }
}
