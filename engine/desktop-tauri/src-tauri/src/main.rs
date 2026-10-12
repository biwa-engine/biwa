//! Biwa のゲームをデスクトップアプリとして包む Tauri のシェル。
//!
//! 中身は `biwa build --platform desktop` が作った Web の静的ビルドで、
//! このシェルはそれを OS の WebView に表示するだけである。
//! ゲームごとの値 (名前・識別子・静的ビルドの場所) は、
//! ビルド時に `tauri build --config` で外から与える。
//!
//! 起動時の引数 (検証用):
//!   - `--probe` (または `BIWA_PROBE=1`): ゲームの代わりに WebView の検証ページを開く。
//!   - `--localhost` (または `BIWA_LOCALHOST=1`): 静的ビルドを Tauri の独自スキーム
//!     (`tauri://` / `http://tauri.localhost`) ではなく `http://localhost:<port>` で配る。
//!     独自スキームでは cross-origin isolation が効かない WebView のための逃げ道。

// Windows の配布版でコンソールウィンドウを出さない。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{WebviewUrl, WebviewWindowBuilder};

/// WebView の機能を調べる検証ページ (`biwa build` が静的ビルドに同梱する)。
const PROBE_PAGE: &str = "__biwa_probe/index.html";
const GAME_PAGE: &str = "index.html";

/// wasm を動かすのに要る cross-origin isolation のヘッダ。
/// 独自スキームでは `tauri.conf.json` の `app.security.headers` が同じものを付ける。
const CROSS_ORIGIN_ISOLATION: [(&str, &str); 2] = [
    ("Cross-Origin-Opener-Policy", "same-origin"),
    ("Cross-Origin-Embedder-Policy", "require-corp"),
];

fn main() {
    let probe = flag("--probe", "BIWA_PROBE");
    let localhost = flag("--localhost", "BIWA_LOCALHOST");
    let page = if probe { PROBE_PAGE } else { GAME_PAGE };

    enable_shared_array_buffer();

    let mut builder = tauri::Builder::default();

    let url = if localhost {
        let port = free_port();
        builder = builder.plugin(
            tauri_plugin_localhost::Builder::new(port)
                .on_request(|_, response| {
                    for (name, value) in CROSS_ORIGIN_ISOLATION {
                        response.add_header(name, value);
                    }
                })
                .build(),
        );
        let url = format!("http://localhost:{port}/{page}");
        WebviewUrl::External(url.parse().expect("the localhost URL must be valid"))
    } else {
        WebviewUrl::App(page.into())
    };

    builder
        .setup(move |app| {
            let title = app
                .config()
                .product_name
                .clone()
                .unwrap_or_else(|| "Biwa".to_string());

            WebviewWindowBuilder::new(app, "main", url.clone())
                .title(title)
                .inner_size(1280.0, 720.0)
                .min_inner_size(640.0, 360.0)
                .build()?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Biwa desktop shell");
}

/// 引数か環境変数のどちらかで立つフラグ。
fn flag(arg: &str, env: &str) -> bool {
    std::env::args().any(|a| a == arg) || std::env::var_os(env).is_some_and(|v| !v.is_empty())
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

/// 空いているポートを OS に選ばせる。
fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .expect("failed to find a free port for the localhost server")
}
