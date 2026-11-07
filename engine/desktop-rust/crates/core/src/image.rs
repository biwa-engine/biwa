//! 画像の受け渡し。
//!
//! エンジンは画像を番号 ([`ImageHandle`]) で扱い、読み込みは依頼するだけである ([`ImageRequest`])。
//! ファイルを読んでデコードするのも、GPU に上げるのも別の crate の仕事で、
//! エンジンは「元の大きさ」が分かれば足りる (大きさを指定しない画像の大きさを決めるため)。

/// 画像の番号。同じパスの画像には同じ番号が振られる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageHandle(pub u32);

/// 画像を読んでほしいという依頼。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRequest {
    pub image: ImageHandle,
    /// パッケージの `assets/` を基準とした相対パス (検査済み。`/` 区切り)。
    pub path: String,
}

/// デコードした画像。RGBA 8 bit (sRGB、アルファは乗算していない)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` バイト。
    pub rgba: Vec<u8>,
}

/// `.biwa` が書いたアセットのパスを検査して均す。
///
/// パスはパッケージの `assets/` を基準とした相対パスで、`assets/` 自体は含まない
/// (`engine/nodejs/src/engine/api/assets.ts` と同じ規約)。
/// `..` で抜けたり絶対パスを書いたりすると、`assets/` に置くという規約そのものが意味を失うので拒む。
pub fn normalize_asset_path(path: &str) -> Result<String, String> {
    let mut segments = Vec::new();
    for segment in path.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => return Err(format!("asset path must stay inside `assets/`: \"{path}\"")),
            s if s.contains(':') => {
                return Err(format!("asset path must be relative to `assets/`: \"{path}\""));
            }
            s => segments.push(s),
        }
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(format!("asset path must be relative to `assets/`: \"{path}\""));
    }
    if segments.is_empty() {
        return Err(format!("empty asset path: \"{path}\""));
    }
    Ok(segments.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_rejects_escapes() {
        assert_eq!(normalize_asset_path("bg/room.png").unwrap(), "bg/room.png");
        assert_eq!(normalize_asset_path("./bg//room.png").unwrap(), "bg/room.png");
        assert!(normalize_asset_path("../secret.png").is_err());
        assert!(normalize_asset_path("bg/../../x.png").is_err());
        assert!(normalize_asset_path("/etc/passwd").is_err());
        assert!(normalize_asset_path("C:/x.png").is_err());
        assert!(normalize_asset_path("").is_err());
    }
}
