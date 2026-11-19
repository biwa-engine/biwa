//! `biwa dev --platform web` の開発用 HTTP サーバ (Node.js も Vite も使わない)。
//!
//! ツールチェインのビルド済みの Web runtime に、開発中のゲームを添えて配る。配り方は `biwa build --platform web`
//! の出力 (`dist/web/`) と同じ形で、ファイルを写さずにその場から返す:
//!
//! | パス                 | 中身                                                                       |
//! | -------------------- | -------------------------------------------------------------------------- |
//! | `/biwa-game.json`    | ゲームの情報。リクエストのたびに作る (`buildId` は wasm の内容、`dev: true`) |
//! | `/game.wasm`         | コンパイラの出力 (`.biwa_build/wasm/<package>.wasm`)                       |
//! | `/assets/...`        | パッケージの `assets/`                                                     |
//! | `/__biwa_dev/events` | 建て直しの知らせ (Server-Sent Events。`reload` でページを読み直させる)     |
//! | それ以外             | Web runtime (`<toolchain>/runtime/web/`)                                   |
//!
//! - wasm の実行に `SharedArrayBuffer` が要るので、すべての応答に COOP / COEP / CORP を付ける。
//! - 開発用なので 127.0.0.1 だけで待ち受け、キャッシュさせない。
//! - 音や動画のシークのために、1 つの範囲の `Range` に答える。
//! - 根の外を指すパス (`..` など) は拒む。
//!
//! 1 つの接続で 1 つのリクエストだけを扱う (`Connection: close`)。開発用にはこれで足りる。

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};

/// ゲームの情報のパス (`engine/nodejs/src/game/runtime-entry.ts`)。
const GAME_INFO_PATH: &str = "biwa-game.json";
/// wasm のパス (同上)。
const GAME_WASM_PATH: &str = "game.wasm";
/// アセットのパスの先頭。
const ASSETS_PREFIX: &str = "assets";
/// 建て直しの知らせのパス (同上)。
const EVENTS_PATH: &str = "__biwa_dev/events";
/// 知らせの接続が生きているかを確かめる間隔 (切れた接続のスレッドを片付けるため)。
const EVENTS_PING: Duration = Duration::from_secs(15);
/// リクエストのヘッダの大きさの上限。
const MAX_HEADER: usize = 16 * 1024;
/// ポートが塞がっていたときに、いくつ先まで試すか。
const PORT_ATTEMPTS: u16 = 20;

/// 配るもの。
pub struct Site {
    /// ビルド済みの Web runtime のディレクトリ。
    pub web_runtime: PathBuf,
    /// コンパイラの出力。
    pub wasm: PathBuf,
    /// パッケージの `assets/`。
    pub assets: PathBuf,
    /// `biwa-game.json` の `buildId` と `dev` 以外 (名前・タイトルなど)。
    pub info: serde_json::Map<String, serde_json::Value>,
}

/// 動いているサーバ。
pub struct DevServer {
    port: u16,
    clients: Arc<Mutex<Vec<Sender<()>>>>,
}

impl DevServer {
    /// `port` (塞がっていれば次) で待ち受けを始める。`exact` なら `port` だけを試す。
    pub fn start(site: Site, port: u16, exact: bool) -> Result<Self> {
        let attempts = if exact { 1 } else { PORT_ATTEMPTS };
        let (listener, port) = (0..attempts)
            .filter_map(|i| port.checked_add(i))
            .find_map(|p| {
                TcpListener::bind((Ipv4Addr::LOCALHOST, p))
                    .ok()
                    .map(|l| (l, p))
            })
            .with_context(|| {
                if exact {
                    format!("port {port} is in use")
                } else {
                    format!(
                        "ports {port}..{} are in use",
                        port.saturating_add(attempts - 1)
                    )
                }
            })?;

        let site = Arc::new(site);
        let clients: Arc<Mutex<Vec<Sender<()>>>> = Arc::default();
        let accept_clients = clients.clone();
        std::thread::Builder::new()
            .name("biwa-dev-server".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let site = site.clone();
                    let clients = accept_clients.clone();
                    let _ = std::thread::Builder::new()
                        .name("biwa-dev-request".into())
                        .spawn(move || handle(stream, &site, &clients));
                }
            })
            .context("failed to start the dev server thread")?;
        Ok(Self { port, clients })
    }

    pub fn url(&self) -> String {
        format!("http://localhost:{}/", self.port)
    }

    /// 開いているページに読み直させる。
    pub fn reload(&self) {
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        clients.retain(|client| client.send(()).is_ok());
    }
}

/// 1 つの接続を扱う。失敗は接続を閉じるだけ (ブラウザが中断したなど)。
fn handle(stream: TcpStream, site: &Site, clients: &Mutex<Vec<Sender<()>>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let Ok(request) = Request::read(&stream) else {
        return;
    };
    let mut stream = stream;
    let _ = respond(&mut stream, &request, site, clients);
}

struct Request {
    method: String,
    /// パーセントエンコーディングを戻したパスの要素 (`/` で区切ったもの。空と `.` は除く)。
    /// 根の外を指す要素 (`..` など) があれば `None`。
    segments: Option<Vec<String>>,
    range: Option<String>,
}

impl Request {
    fn read(stream: &TcpStream) -> Result<Self> {
        let mut reader = BufReader::new(stream.take(MAX_HEADER as u64));
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let mut parts = line.split_whitespace();
        let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
            bail!("malformed request line");
        };
        let mut range = None;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':')
                && name.trim().eq_ignore_ascii_case("range")
            {
                range = Some(value.trim().to_string());
            }
        }
        let path = target.split(['?', '#']).next().unwrap_or("/");
        Ok(Self {
            method: method.to_string(),
            segments: segments(path),
            range,
        })
    }
}

/// URL のパスを要素に分ける。根の外を指すもの・OS のパスとして危ないものがあれば `None`。
fn segments(path: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for raw in path.split('/') {
        let segment = percent_decode(raw)?;
        match segment.as_str() {
            "" | "." => continue,
            ".." => return None,
            s if s.contains(['\\', ':', '\0']) => return None,
            _ => out.push(segment),
        }
    }
    Some(out)
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn respond(
    stream: &mut TcpStream,
    request: &Request,
    site: &Site,
    clients: &Mutex<Vec<Sender<()>>>,
) -> std::io::Result<()> {
    let head_only = match request.method.as_str() {
        "GET" => false,
        "HEAD" => true,
        _ => return simple(stream, 405, "Method Not Allowed"),
    };
    let Some(segments) = &request.segments else {
        return simple(stream, 403, "Forbidden");
    };
    let joined = segments.join("/");

    match joined.as_str() {
        EVENTS_PATH => {
            let (tx, rx) = mpsc::channel();
            clients.lock().unwrap_or_else(|e| e.into_inner()).push(tx);
            events(stream, &rx)
        }
        GAME_INFO_PATH => {
            let mut info = site.info.clone();
            if let Ok(wasm) = std::fs::read(&site.wasm) {
                info.insert("buildId".into(), crate::files::content_id(&wasm).into());
            }
            info.insert("dev".into(), true.into());
            let body = serde_json::to_vec_pretty(&info).unwrap_or_default();
            bytes(stream, "application/json", &body, head_only)
        }
        GAME_WASM_PATH => file(stream, &site.wasm, request.range.as_deref(), head_only),
        _ => {
            let path = match segments.split_first() {
                Some((first, rest)) if first == ASSETS_PREFIX => join(&site.assets, rest),
                _ if segments.is_empty() => site.web_runtime.join("index.html"),
                _ => join(&site.web_runtime, segments),
            };
            file(stream, &path, request.range.as_deref(), head_only)
        }
    }
}

fn join(root: &Path, segments: &[String]) -> PathBuf {
    segments
        .iter()
        .fold(root.to_path_buf(), |path, s| path.join(s))
}

/// どの応答にも付けるヘッダ。
fn common_headers() -> &'static str {
    "Cross-Origin-Opener-Policy: same-origin\r\n\
     Cross-Origin-Embedder-Policy: require-corp\r\n\
     Cross-Origin-Resource-Policy: same-origin\r\n\
     Cache-Control: no-cache\r\n\
     Connection: close\r\n"
}

fn simple(stream: &mut TcpStream, status: u16, reason: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\n{}Content-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n{reason}",
        common_headers(),
        reason.len()
    )
}

fn bytes(
    stream: &mut TcpStream,
    content_type: &str,
    body: &[u8],
    head_only: bool,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\n{}Content-Type: {content_type}\r\nContent-Length: {}\r\n\r\n",
        common_headers(),
        body.len()
    )?;
    if !head_only {
        stream.write_all(body)?;
    }
    Ok(())
}

fn file(
    stream: &mut TcpStream,
    path: &Path,
    range: Option<&str>,
    head_only: bool,
) -> std::io::Result<()> {
    let Ok(mut f) = std::fs::File::open(path) else {
        return simple(stream, 404, "Not Found");
    };
    let len = match f.metadata() {
        Ok(meta) if meta.is_file() => meta.len(),
        _ => return simple(stream, 404, "Not Found"),
    };
    let content_type = content_type(path);

    let (status, start, end) = match range.map(|r| parse_range(r, len)) {
        None => ("200 OK", 0, len),
        Some(Some((start, end))) => ("206 Partial Content", start, end),
        Some(None) => {
            return write!(
                stream,
                "HTTP/1.1 416 Range Not Satisfiable\r\n{}Content-Range: bytes */{len}\r\nContent-Length: 0\r\n\r\n",
                common_headers()
            );
        }
    };
    let mut head = format!(
        "HTTP/1.1 {status}\r\n{}Content-Type: {content_type}\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\n",
        common_headers(),
        end - start
    );
    if status.starts_with("206") {
        head.push_str(&format!(
            "Content-Range: bytes {start}-{}/{len}\r\n",
            end - 1
        ));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if head_only {
        return Ok(());
    }
    f.seek(SeekFrom::Start(start))?;
    std::io::copy(&mut f.take(end - start), stream)?;
    Ok(())
}

/// `bytes=<start>-<end>` (1 つの範囲) を `[start, end)` にする。満たせなければ `None`。
fn parse_range(header: &str, len: u64) -> Option<(u64, u64)> {
    let spec = header.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let (start, end) = match (start.trim(), end.trim()) {
        // 末尾から n バイト。
        ("", n) => {
            let n: u64 = n.parse().ok()?;
            (len.saturating_sub(n), len)
        }
        (s, "") => (s.parse().ok()?, len),
        (s, e) => (
            s.parse().ok()?,
            e.parse::<u64>().ok()?.saturating_add(1).min(len),
        ),
    };
    (start < end && start < len).then_some((start, end))
}

/// 建て直しの知らせを流し続ける。接続が切れたら終わる。
fn events(stream: &mut TcpStream, rx: &Receiver<()>) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\n{}Content-Type: text/event-stream\r\n\r\n",
        common_headers()
    )?;
    stream.flush()?;
    loop {
        match rx.recv_timeout(EVENTS_PING) {
            Ok(()) => stream.write_all(b"event: reload\ndata: \n\n")?,
            // 書けなければ接続が切れている。
            Err(RecvTimeoutError::Timeout) => stream.write_all(b": ping\n\n")?,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        stream.flush()?;
    }
}

fn content_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "ogg" | "oga" => "audio/ogg",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "flac" => "audio/flac",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_paths_and_rejects_escapes() {
        assert_eq!(
            segments("/assets/%E7%94%BB%E5%83%8F.png"),
            Some(vec!["assets".into(), "画像.png".into()])
        );
        assert_eq!(segments("/./a//b/"), Some(vec!["a".into(), "b".into()]));
        assert_eq!(segments("/assets/../secret"), None);
        assert_eq!(segments("/assets/%2e%2e/secret"), None);
        assert_eq!(segments("/a%5Cb"), None);
        assert_eq!(segments("/C:/x"), None);
        assert_eq!(segments("/%zz"), None);
    }

    #[test]
    fn parses_single_ranges() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 100)));
        assert_eq!(parse_range("bytes=900-", 1000), Some((900, 1000)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 1000)));
        assert_eq!(parse_range("bytes=990-2000", 1000), Some((990, 1000)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("bytes=0-1,5-9", 1000), None);
    }
}
