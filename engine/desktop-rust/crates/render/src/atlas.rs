//! グリフのアトラス。アルファだけの 1 枚のテクスチャにグリフの画像を詰める。
//!
//! 初めて見た鍵のときだけ [`GlyphRasterizer`] に画像を頼む。
//! 満杯になったら全部捨てて詰め直す (ノベルゲームの画面に同時に出る字の種類は多くない)。

use std::collections::HashMap;

use biwa_player_core::text::{GlyphKey, GlyphRasterizer};
use etagere::{AtlasAllocator, size2};

/// アトラスの一辺 (ピクセル)。
const SIZE: u32 = 2048;
/// グリフの間の隙間。サンプリングで隣が滲まないように。
const PADDING: i32 = 1;

/// アトラスに載ったグリフ。
#[derive(Debug, Clone, Copy)]
pub(crate) struct AtlasGlyph {
    /// u0, v0, u1, v1。
    pub uv: [f32; 4],
    pub width: f32,
    pub height: f32,
    /// ベースライン上の位置から画像の左上までのずれ。
    pub left: f32,
    pub top: f32,
}

pub(crate) struct GlyphAtlas {
    allocator: AtlasAllocator,
    pub texture: wgpu::Texture,
    /// 画像の無いグリフ (空白など) は `None` で覚えておく。
    glyphs: HashMap<GlyphKey, Option<AtlasGlyph>>,
}

impl GlyphAtlas {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("biwa glyph atlas"),
            size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        Self {
            allocator: AtlasAllocator::new(size2(SIZE as i32, SIZE as i32)),
            texture,
            glyphs: HashMap::new(),
        }
    }

    /// `key` のグリフを返す。まだ載っていなければ画像にして載せる。
    pub fn get(
        &mut self,
        key: GlyphKey,
        rasterizer: &mut dyn GlyphRasterizer,
        queue: &wgpu::Queue,
    ) -> Option<AtlasGlyph> {
        if let Some(glyph) = self.glyphs.get(&key) {
            return *glyph;
        }
        let glyph = rasterizer.rasterize(key).and_then(|image| {
            let size = size2(image.width as i32 + PADDING * 2, image.height as i32 + PADDING * 2);
            let allocation = self.allocator.allocate(size).or_else(|| {
                // 満杯。全部捨てて詰め直す (このフレームで使う他のグリフは後で載せ直される)。
                self.allocator.clear();
                self.glyphs.clear();
                self.allocator.allocate(size)
            })?;
            let x = (allocation.rectangle.min.x + PADDING) as u32;
            let y = (allocation.rectangle.min.y + PADDING) as u32;
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &image.alpha,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(image.width),
                    rows_per_image: Some(image.height),
                },
                wgpu::Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 },
            );
            let s = SIZE as f32;
            Some(AtlasGlyph {
                uv: [
                    x as f32 / s,
                    y as f32 / s,
                    (x + image.width) as f32 / s,
                    (y + image.height) as f32 / s,
                ],
                width: image.width as f32,
                height: image.height as f32,
                left: image.left as f32,
                top: image.top as f32,
            })
        });
        self.glyphs.insert(key, glyph);
        glyph
    }
}
