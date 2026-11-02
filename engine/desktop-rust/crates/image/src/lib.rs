//! 画像のアセットを読み込んでデコードする (image crate)。
//!
//! 外から見えるのは [`ImageLoader`] と、`core::image` の型 ([`ImageRequest`] / [`DecodedImage`]) だけである。
//!
//! 読み込みとデコードは専用のスレッドで行う。大きな背景の JPEG のデコードには数十 ms かかることがあり、
//! メインスレッドでやると描画が止まる。TypeScript 版でも読み込みは非同期で、
//! オブジェクトは先に作り、画像が届いてから描いていた (`CanvasObjects.create`)。

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

use biwa_player_core::image::{DecodedImage, ImageHandle, ImageRequest};

/// 読み込みの結果。
pub type LoadResult = (ImageHandle, Result<DecodedImage, String>);

/// 画像を読む者。依頼を受けて裏のスレッドで読み、結果をためておく。
pub struct ImageLoader {
    requests: Sender<ImageRequest>,
    results: Receiver<LoadResult>,
}

impl ImageLoader {
    /// `assets` (パッケージの `assets/` に当たるディレクトリ) から読む。
    ///
    /// `notify` は結果が 1 つ届くたびに (裏のスレッドから) 呼ばれる。イベントループを起こすのに使う。
    pub fn new(assets: PathBuf, notify: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let (requests, request_rx) = mpsc::channel::<ImageRequest>();
        let (result_tx, results) = mpsc::channel();
        std::thread::Builder::new().name("biwa-image".into()).spawn(move || {
            for request in request_rx {
                let result = load(&assets, &request.path);
                if result_tx.send((request.image, result)).is_err() {
                    return;
                }
                notify();
            }
        })?;
        Ok(Self { requests, results })
    }

    /// 読み込みを依頼する。
    pub fn request(&self, request: ImageRequest) {
        // 送れないのは裏のスレッドが終わったあとで、そのときは結果も来ない。
        let _ = self.requests.send(request);
    }

    /// 届いている結果を 1 つ取る。
    pub fn try_recv(&self) -> Option<LoadResult> {
        self.results.try_recv().ok()
    }
}

/// `assets/<path>` を読んで RGBA にする。
fn load(assets: &Path, path: &str) -> Result<DecodedImage, String> {
    let file = assets.join(path);
    let image = image::open(&file)
        .map_err(|e| format!("failed to load \"{path}\" (looked for {}): {e}", file.display()))?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Ok(DecodedImage { width, height, rgba: image.into_raw() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_on_a_background_thread() {
        let dir = std::env::temp_dir().join(format!("biwa-image-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("bg")).unwrap();
        image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255]))
            .save(dir.join("bg/one.png"))
            .unwrap();

        let (tx, rx) = mpsc::channel();
        let loader = ImageLoader::new(dir.clone(), move || {
            let _ = tx.send(());
        })
        .unwrap();
        loader.request(ImageRequest { image: ImageHandle(1), path: "bg/one.png".into() });
        loader.request(ImageRequest { image: ImageHandle(2), path: "missing.png".into() });
        rx.recv().unwrap();
        rx.recv().unwrap();

        let (image, ok) = loader.try_recv().unwrap();
        assert_eq!(image, ImageHandle(1));
        let ok = ok.unwrap();
        assert_eq!((ok.width, ok.height), (3, 2));
        assert_eq!(&ok.rgba[..4], &[10, 20, 30, 255]);

        let (image, missing) = loader.try_recv().unwrap();
        assert_eq!(image, ImageHandle(2));
        assert!(missing.unwrap_err().contains("missing.png"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
