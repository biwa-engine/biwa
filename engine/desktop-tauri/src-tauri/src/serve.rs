//! ゲームのデータ (`game/`) をファイルとして配る。
//!
//! 独自スキーム (`biwa://`) と `--localhost` の HTTP サーバの両方がこれを使う。
//! どちらの経路でも、wasm を動かすのに要る cross-origin isolation のヘッダを付ける。

use std::path::{Component, Path, PathBuf};

/// wasm を動かすのに要る cross-origin isolation のヘッダ。
///
/// ブロッキング syscall に `SharedArrayBuffer` を使うため。
/// 同じオリジンのものしか読まないので、CORP も same-origin で足りる。
pub const CROSS_ORIGIN_ISOLATION: [(&str, &str); 3] = [
    ("Cross-Origin-Opener-Policy", "same-origin"),
    ("Cross-Origin-Embedder-Policy", "require-corp"),
    ("Cross-Origin-Resource-Policy", "same-origin"),
];

/// 配る応答。HTTP のライブラリに依らない形で持つ。
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

/// URL のパス (`/index.html` など) と `Range` ヘッダから応答を作る。
pub fn serve(root: &Path, url_path: &str, range: Option<&str>) -> Reply {
    let Some(path) = resolve(root, url_path) else {
        return error(404);
    };

    let body = match std::fs::read(&path) {
        Ok(body) => body,
        Err(_) => return error(404),
    };

    let mut headers = common_headers(mime_type(&path));

    // 音や動画のシークは Range で来る。WebKit は Range に答えないと再生しないことがある。
    if let Some((start, end)) = range.and_then(|r| parse_range(r, body.len())) {
        headers.push((
            "Content-Range",
            format!("bytes {start}-{end}/{}", body.len()),
        ));
        return Reply {
            status: 206,
            headers,
            body: body[start..=end].to_vec(),
        };
    }

    Reply {
        status: 200,
        headers,
        body,
    }
}

fn common_headers(content_type: &str) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        ("Content-Type", content_type.to_string()),
        ("Accept-Ranges", "bytes".to_string()),
        // ゲームのデータは配布物の中で固定なので、キャッシュの心配は要らないが、
        // `biwa dev` で中身が差し替わることを見越して毎回確かめさせる。
        ("Cache-Control", "no-cache".to_string()),
    ];
    for (name, value) in CROSS_ORIGIN_ISOLATION {
        headers.push((name, value.to_string()));
    }
    headers
}

fn error(status: u16) -> Reply {
    Reply {
        status,
        headers: common_headers("text/plain; charset=utf-8"),
        body: format!("{status}").into_bytes(),
    }
}

/// URL のパスを `root` の下のファイルに解く。`root` の外へは出さない。
fn resolve(root: &Path, url_path: &str) -> Option<PathBuf> {
    let decoded = percent_decode(url_path.split(['?', '#']).next().unwrap_or(""))?;
    let relative = decoded.trim_start_matches('/');
    let relative = if relative.is_empty() || relative.ends_with('/') {
        format!("{relative}index.html")
    } else {
        relative.to_string()
    };

    let mut path = root.to_path_buf();
    for component in Path::new(&relative).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            // `..` や絶対パスで `root` を抜けようとするものは拒む。
            _ => return None,
        }
    }
    path.is_file().then_some(path)
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `bytes=<start>-<end>` / `bytes=<start>-` / `bytes=-<suffix>` を解く。
/// 複数の範囲や解けないものは無視して全体を返す (200)。
fn parse_range(header: &str, len: usize) -> Option<(usize, usize)> {
    if len == 0 {
        return None;
    }
    let spec = header.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let (start, end) = match (start.trim(), end.trim()) {
        ("", suffix) => {
            let suffix: usize = suffix.parse().ok()?;
            (len.saturating_sub(suffix), len - 1)
        }
        (start, "") => (start.parse().ok()?, len - 1),
        (start, end) => (start.parse().ok()?, end.parse::<usize>().ok()?.min(len - 1)),
    };
    (start <= end && start < len).then_some((start, end))
}

fn mime_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-9", 100), Some((0, 9)));
        assert_eq!(parse_range("bytes=90-", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=50-500", 100), Some((50, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=0-1,5-6", 100), None);
    }

    #[test]
    fn stays_inside_root() {
        let root = std::env::temp_dir().join("biwa-player-serve-test");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("index.html"), "x").unwrap();
        std::fs::write(root.join("sub/a b.png"), "x").unwrap();

        assert_eq!(resolve(&root, "/"), Some(root.join("index.html")));
        assert_eq!(
            resolve(&root, "/sub/a%20b.png?v=1"),
            Some(root.join("sub/a b.png"))
        );
        assert_eq!(resolve(&root, "/../etc/passwd"), None);
        assert_eq!(resolve(&root, "/sub/../../x"), None);
        assert_eq!(resolve(&root, "/missing.js"), None);
    }
}
