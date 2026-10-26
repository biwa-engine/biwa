//! Biwa のゲームを動かすデスクトップのプレイヤー (Tauri v2)。
//!
//! ゲームを焼き込まない汎用の実行ファイルで、Biwa の CI が OS ごとに 1 度だけビルドする。
//! `biwa build` は、これにゲームのデータ (`game/`) を添えて配布物を組み立てる
//! (利用者の手元で Rust のビルドは走らない。`docs/biwa-prebuilt-player.md`)。
//!
//! ゲームのデータの場所:
//!   - Windows / Linux: 実行ファイルと同じディレクトリの `game/`
//!   - macOS: `<name>.app/Contents/Resources/game/`
//!   - `--game-dir <DIR>` (または環境変数 `BIWA_GAME_DIR`) で上書きできる
//!
//! ゲームのデータの配り方 (どちらでも COOP / COEP を付ける):
//!   - 独自スキーム `biwa://localhost/` (Windows は `http://biwa.localhost/`)。Windows / Linux の既定。
//!   - `http://localhost:<port>/` (プレイヤーの中の HTTP サーバ)。macOS の既定。
//!     WKWebView は独自スキームのページで `SharedArrayBuffer` を出さない
//!     (`crossOriginIsolated` は true になるのに。macOS 27 / Safari 22625 で確認)。
//!     HTTP で配れば出る。`docs/desktop-build.md` §4。
//!
//! 起動時の引数 (検証用):
//!   - `--probe` (または `BIWA_PROBE=1`): ゲームの代わりに WebView の検証ページを開く。
//!   - `--localhost` (または `BIWA_LOCALHOST=1`): どの OS でも localhost で配る。
//!   - `--custom-scheme` (または `BIWA_CUSTOM_SCHEME=1`): どの OS でも独自スキームで配る。

// Windows の配布版でコンソールウィンドウを出さない。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod serve;

use std::path::{Path, PathBuf};

use serde::Deserialize;
use tauri::{WebviewUrl, WebviewWindowBuilder};

/// ゲームのデータを配る独自スキーム。
const SCHEME: &str = "biwa";
/// WebView の機能を調べる検証ページ (`biwa build` が `game/` に同梱する)。
const PROBE_PAGE: &str = "__biwa_probe/index.html";
const GAME_PAGE: &str = "index.html";
/// `game/` の中の、ゲームの情報を書いたファイル (`biwa build` が書く)。
const GAME_INFO_FILE: &str = "biwa-game.json";

/// `biwa-game.json`。プレイヤーがウィンドウを作るのに要る情報。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GameInfo {
    /// パッケージ名。localhost で配るときのポートを決めるのに使う。
    name: Option<String>,
    /// ウィンドウのタイトル。
    title: Option<String>,
    /// ウィンドウの最初の大きさ (論理ピクセル)。
    width: Option<f64>,
    height: Option<f64>,
}

fn main() {
    let probe = flag("--probe", "BIWA_PROBE");
    let localhost = if flag("--custom-scheme", "BIWA_CUSTOM_SCHEME") {
        false
    } else {
        flag("--localhost", "BIWA_LOCALHOST") || LOCALHOST_BY_DEFAULT
    };
    let page = if probe { PROBE_PAGE } else { GAME_PAGE };

    enable_shared_array_buffer();

    let game_dir = game_dir();
    let info = game_dir.as_deref().map(read_game_info).unwrap_or_default();

    let url = match &game_dir {
        // データが無ければ、同梱のプレースホルダ (案内のページ) を出す。
        None => WebviewUrl::App("index.html".into()),
        Some(dir) if localhost => {
            let key = info
                .name
                .as_deref()
                .or(info.title.as_deref())
                .unwrap_or("biwa");
            let port = serve_localhost(dir.clone(), key);
            external(&format!("http://localhost:{port}/{page}"))
        }
        Some(_) => external(&format!("{}{page}", custom_protocol_origin())),
    };

    let protocol_root = game_dir.clone().unwrap_or_default();

    tauri::Builder::default()
        .register_uri_scheme_protocol(SCHEME, move |_ctx, request| {
            let range = request.headers().get("range").and_then(|v| v.to_str().ok());
            let reply = serve::serve(&protocol_root, request.uri().path(), range);

            let mut response = tauri::http::Response::builder().status(reply.status);
            for (name, value) in reply.headers {
                response = response.header(name, value);
            }
            response
                .body(reply.body)
                .expect("the response headers must be valid")
        })
        .setup(move |app| {
            let title = info.title.clone().unwrap_or_else(|| "Biwa".to_string());

            WebviewWindowBuilder::new(app, "main", url.clone())
                .title(title)
                .inner_size(info.width.unwrap_or(1280.0), info.height.unwrap_or(720.0))
                .min_inner_size(640.0, 360.0)
                .build()?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Biwa player");
}

/// 既定で localhost で配るか。macOS の WKWebView は独自スキームで `SharedArrayBuffer` を出さない。
const LOCALHOST_BY_DEFAULT: bool = cfg!(target_os = "macos");

/// 引数か環境変数のどちらかで立つフラグ。
fn flag(arg: &str, env: &str) -> bool {
    std::env::args().any(|a| a == arg) || std::env::var_os(env).is_some_and(|v| !v.is_empty())
}

/// ゲームのデータのディレクトリを探す。見つからなければ `None`。
fn game_dir() -> Option<PathBuf> {
    let explicit = arg_value("--game-dir")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("BIWA_GAME_DIR").map(PathBuf::from));
    if let Some(dir) = explicit {
        if dir.is_dir() {
            return Some(dir);
        }
        eprintln!("biwa-player: game directory not found: {}", dir.display());
        return None;
    }

    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let candidates = [
        exe_dir.join("game"),
        // macOS: <name>.app/Contents/MacOS/<exe> から Contents/Resources/game へ。
        exe_dir.join("..").join("Resources").join("game"),
    ];
    let found = candidates
        .into_iter()
        .find(|dir| dir.join(GAME_PAGE).is_file());
    if found.is_none() {
        eprintln!(
            "biwa-player: no game data next to {} (expected a `game/` directory)",
            exe.display()
        );
    }
    found
}

fn arg_value(name: &str) -> Option<String> {
    let mut args = std::env::args();
    while let Some(arg) = args.next() {
        if arg == name {
            return args.next();
        }
        if let Some(value) = arg.strip_prefix(&format!("{name}=")) {
            return Some(value.to_string());
        }
    }
    None
}

fn read_game_info(dir: &Path) -> GameInfo {
    std::fs::read_to_string(dir.join(GAME_INFO_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn external(url: &str) -> WebviewUrl {
    WebviewUrl::External(url.parse().expect("the game URL must be valid"))
}

/// 独自スキームのオリジン。
///
/// Windows (WebView2) は独自スキームを `http://<scheme>.localhost/` として扱う。
#[cfg(windows)]
fn custom_protocol_origin() -> String {
    format!("http://{SCHEME}.localhost/")
}

#[cfg(not(windows))]
fn custom_protocol_origin() -> String {
    format!("{SCHEME}://localhost/")
}

/// `game/` を `http://localhost:<port>` で配るサーバを裏で立てる。
///
/// ポートはゲームごとに決まった値を使う。オリジン (`http://localhost:<port>`) が
/// 起動のたびに変わると、WebView のストレージ (localStorage・IndexedDB) が
/// 前回のものを読めなくなるため。塞がっていれば次を試し、だめなら空いているものにする。
fn serve_localhost(root: PathBuf, key: &str) -> u16 {
    let preferred = preferred_port(key);
    let server = (0..PORT_ATTEMPTS)
        .map(|i| preferred_port_offset(preferred, i))
        .find_map(|port| tiny_http::Server::http(("127.0.0.1", port)).ok())
        .or_else(|| {
            eprintln!("biwa-player: ports from {preferred} are in use; using a random port");
            tiny_http::Server::http(("127.0.0.1", 0)).ok()
        })
        .expect("failed to start the localhost server");
    let port = server
        .server_addr()
        .to_ip()
        .expect("the localhost server must listen on an IP address")
        .port();

    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            let range = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("Range"))
                .map(|h| h.value.as_str().to_string());
            let reply = serve::serve(&root, request.url(), range.as_deref());

            let mut response =
                tiny_http::Response::from_data(reply.body).with_status_code(reply.status);
            for (name, value) in reply.headers {
                if let Ok(header) = tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes())
                {
                    response.add_header(header);
                }
            }
            let _ = request.respond(response);
        }
    });

    port
}

/// ゲームごとのポートを選ぶ範囲。動的ポート (49152-65535) を避け、登録ポートの上の方を使う。
const PORT_BASE: u16 = 40000;
const PORT_RANGE: u16 = 9000;
/// 決まったポートが塞がっていたときに、次を試す回数。
const PORT_ATTEMPTS: u16 = 8;

/// `key` (パッケージ名) から決まるポート。FNV-1a。
fn preferred_port(key: &str) -> u16 {
    let mut hash: u32 = 0x811c_9dc5;
    for b in key.bytes() {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    PORT_BASE + (hash % u32::from(PORT_RANGE)) as u16
}

fn preferred_port_offset(preferred: u16, i: u16) -> u16 {
    PORT_BASE + (preferred - PORT_BASE + i) % PORT_RANGE
}

/// WebKitGTK (Linux) は、cross-origin isolated なページでも既定では
/// `SharedArrayBuffer` を出さない。JavaScriptCore の実行時オプションで有効にする。
///
/// オプションは WebView のプロセス (WebKitWebProcess) が環境変数から読むので、
/// WebView を作る前に自分の環境に置いておけば引き継がれる。
#[cfg(target_os = "linux")]
fn enable_shared_array_buffer() {
    const OPTION: &str = "JSC_useSharedArrayBuffer";
    if std::env::var_os(OPTION).is_none() {
        // SAFETY: スレッドを立てる前 (Tauri を起動する前) に呼んでいる。
        unsafe { std::env::set_var(OPTION, "1") };
    }
}

#[cfg(not(target_os = "linux"))]
fn enable_shared_array_buffer() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_are_stable_and_in_range() {
        let port = preferred_port("test1");
        assert_eq!(port, preferred_port("test1"));
        assert!((PORT_BASE..PORT_BASE + PORT_RANGE).contains(&port));
        assert_ne!(port, preferred_port("test2"));

        // 範囲の端で折り返す。
        let last = PORT_BASE + PORT_RANGE - 1;
        assert_eq!(preferred_port_offset(last, 1), PORT_BASE);
    }
}
