//! 文字の整形 (parley) とグリフのラスタライズ (swash)。
//!
//! 外から見えるのは [`TextSystem`] が実装する `core::text` のトレイト ([`TextShaper`] /
//! [`GlyphRasterizer`]) だけで、parley / swash の型はこの crate の外に出さない。
//!
//! - 整形: 断片の列を 1 つの段落として parley に渡す。断片の番号は parley の「ブラシ」に持たせ、
//!   クラスタのバイト位置から「何番目の断片の何文字目か」を引く (文字送りがそれで隠す)。
//! - フォント: 今はシステムの sans-serif を日本語のロケールで使う
//!   (ゲームに同梱したフォントの登録は後で。`docs/biwa-distribution-targets.md` のフォントの節)。
//! - 改行: UAX #14 に禁則処理 (`kinsoku.rs`) を重ねる。
//! - ラスタライズ: swash でアルファだけの画像にする。可変フォントの座標と疑似ボールドは、
//!   整形したときの情報を鍵ごとに覚えておいて使う。

mod kinsoku;

use std::collections::HashMap;

use biwa_player_core::text::{
    GlyphImage, GlyphKey, GlyphRasterizer, ParagraphStyle, ShapedGlyph, ShapedText, TextShaper,
    TextSpan,
};
use parley::{
    Alignment, AlignmentOptions, CHROMIUM_LINE_BREAK_OVERRIDE, FontContext, FontData, FontFamily,
    FontFamilyName, FontWeight, GenericFamily, Language, Layout, LayoutContext, LineBreakContext,
    LineBreakOverrideFn, LineHeight, PositionedLayoutItem, StyleProperty,
};
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::Format;

/// 断片の番号。parley のブラシとして運ぶ。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct SpanTag(usize);

/// 改行位置の上書き: 禁則で止めるもの以外は Chromium と同じ振る舞いにする。
static LINE_BREAKS: &LineBreakOverrideFn =
    &(line_breaks as fn(LineBreakContext) -> Option<bool>);

fn line_breaks(cx: LineBreakContext) -> Option<bool> {
    kinsoku::line_break_override(cx.before, cx.after).or_else(|| CHROMIUM_LINE_BREAK_OVERRIDE(cx))
}

/// 同じ鍵のグリフを描き直すのに要る、整形したときの情報。
#[derive(Debug, Clone)]
struct Variant {
    /// 可変フォントの正規化座標 (太さなど)。
    coords: Vec<i16>,
    /// 太さの合うフォントが無く、輪郭を太らせて代用する。
    embolden: bool,
}

/// 文字の整形とラスタライズを受け持つ。メインスレッドに 1 つ置く。
pub struct TextSystem {
    fonts: FontContext,
    layouts: LayoutContext<SpanTag>,
    scaler: ScaleContext,
    /// 整形で出会ったフォント。添字が [`GlyphKey::font`]。
    font_data: Vec<FontData>,
    font_ids: HashMap<(u64, u32), u32>,
    variants: HashMap<(u32, u16), Variant>,
    japanese: Language,
}

impl Default for TextSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl TextSystem {
    /// システムのフォントを読み込む。
    pub fn new() -> Self {
        Self {
            fonts: FontContext::new(),
            layouts: LayoutContext::new(),
            scaler: ScaleContext::new(),
            font_data: Vec::new(),
            font_ids: HashMap::new(),
            variants: HashMap::new(),
            japanese: Language::parse("ja").expect("`ja` is a valid language tag"),
        }
    }

    fn font_id(&mut self, font: &FontData) -> u32 {
        let key = (font.data.id(), font.index);
        if let Some(id) = self.font_ids.get(&key) {
            return *id;
        }
        let id = self.font_data.len() as u32;
        self.font_data.push(font.clone());
        self.font_ids.insert(key, id);
        id
    }
}

impl TextShaper for TextSystem {
    fn shape(&mut self, spans: &[TextSpan], paragraph: ParagraphStyle) -> ShapedText {
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        if text.is_empty() {
            return ShapedText::default();
        }

        // 断片ごとのバイトの範囲と、バイト位置 → 断片の中の文字番号の対応。
        let mut ranges = Vec::with_capacity(spans.len());
        let mut char_index_at = vec![0usize; text.len() + 1];
        let mut start = 0;
        for span in spans {
            let end = start + span.text.len();
            for (i, (byte, _)) in span.text.char_indices().enumerate() {
                char_index_at[start + byte] = i;
            }
            ranges.push(start..end);
            start = end;
        }

        let layout: Layout<SpanTag> = {
            // 大きさは物理ピクセルで受け取っているので、parley の側の倍率は 1。
            let mut builder = self.layouts.ranged_builder(&mut self.fonts, &text, 1.0, true);
            builder.set_line_break_override(Some(LINE_BREAKS));
            builder.push_default(StyleProperty::FontFamily(FontFamily::Single(
                FontFamilyName::Generic(GenericFamily::SansSerif),
            )));
            builder.push_default(StyleProperty::Locale(Some(self.japanese.clone())));
            builder.push_default(StyleProperty::LineHeight(match paragraph.line_height {
                biwa_player_core::text::LineHeight::Normal => LineHeight::MetricsRelative(1.0),
                biwa_player_core::text::LineHeight::Relative(r) => LineHeight::FontSizeRelative(r),
            }));
            for (i, (span, range)) in spans.iter().zip(&ranges).enumerate() {
                builder.push(StyleProperty::Brush(SpanTag(i)), range.clone());
                builder.push(StyleProperty::FontSize(span.style.size), range.clone());
                builder.push(
                    StyleProperty::FontWeight(FontWeight::new(f32::from(span.style.weight))),
                    range.clone(),
                );
                if let Some(family) = &span.style.family {
                    // CSS の font-family と同じ書き方 (候補をカンマで並べる)。見つからなければ sans-serif に落ちる。
                    let source = format!("{family}, sans-serif");
                    builder.push(StyleProperty::FontFamily(FontFamily::Source(source.into())), range.clone());
                }
            }
            builder.build(&text)
        };
        let mut layout = layout;
        let max_width = paragraph.max_width.is_finite().then(|| paragraph.max_width.max(1.0));
        layout.break_all_lines(max_width);
        if paragraph.center {
            layout.align(Alignment::Center, AlignmentOptions::default());
        }

        let mut glyphs = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let run = glyph_run.run();
                let font = self.font_id(run.font());
                let size = run.font_size();

                let mut x = glyph_run.offset();
                let baseline = glyph_run.baseline();
                // 1 つの run (同じフォント・大きさ) が、書式 (= 断片) ごとに複数のグリフの並びに分かれる。
                // この並びに属するクラスタだけを辿る (断片ごとにブラシが違うので書式の番号で見分けられる)。
                let style = glyph_run.style_index();
                let Some(first) = run.visual_clusters().find(|c| c.style_index() == style) else {
                    continue;
                };
                // 鍵の太さは「要求した」太さにする。選ばれたフォントの太さで鍵を作ると、
                // 太字の無いフォントで普通の字と太字 (疑似ボールド) が同じ鍵になってしまう。
                let weight = spans.get(first.style().brush.0).map_or(400, |s| s.style.weight);
                let coords: Vec<i16> = run.normalized_coords().iter().map(|c| c.to_bits()).collect();
                // 選ばれたフォントが要求より 200 以上細く、太さの軸 (可変フォント) も無いなら輪郭を太らせる。
                // fontique 自身の条件と同じだが、parley の run がそれを伝えないことがある
                // (IPA ゴシックで太字を要求しても `synthesis().embolden()` が false だった)。
                let matched = run.font_attrs().weight.value();
                let embolden = run.synthesis().embolden()
                    || (coords.is_empty() && f32::from(weight) > matched + 200.0);
                self.variants.entry((font, weight)).or_insert(Variant { coords, embolden });
                for cluster in run.visual_clusters().filter(|c| c.style_index() == style) {
                    let byte = cluster.text_range().start;
                    let span = cluster.style().brush.0;
                    let span_start = ranges.get(span).map_or(0, |r| r.start);
                    let char_index = char_index_at[byte.max(span_start)];
                    let color = spans.get(span).map_or_else(Default::default, |s| s.style.color);
                    for glyph in cluster.glyphs() {
                        glyphs.push(ShapedGlyph {
                            key: GlyphKey {
                                font,
                                glyph: glyph.id,
                                size: (size * 64.0).round() as u32,
                                weight,
                            },
                            x: x + glyph.x,
                            y: baseline + glyph.y,
                            color,
                            span,
                            char_index,
                        });
                        x += glyph.advance;
                    }
                }
            }
        }

        ShapedText { glyphs, width: layout.width(), height: layout.height() }
    }
}

impl GlyphRasterizer for TextSystem {
    fn rasterize(&mut self, key: GlyphKey) -> Option<GlyphImage> {
        let font = self.font_data.get(key.font as usize)?;
        let font_ref = swash::FontRef::from_index(font.data.as_ref(), font.index as usize)?;
        let variant = self.variants.get(&(key.font, key.weight));
        let coords = variant.map(|v| v.coords.clone()).unwrap_or_default();
        let mut scaler = self
            .scaler
            .builder(font_ref)
            .size(key.size as f32 / 64.0)
            .hint(true)
            .normalized_coords(coords)
            .build();
        let mut render = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ]);
        render.format(Format::Alpha);
        if variant.is_some_and(|v| v.embolden) {
            // 太らせる幅。Chromium (Skia) の疑似ボールドに近い強さ (小さい字ほど相対的に太く)。
            let size = key.size as f32 / 64.0;
            render.embolden((size / 16.0).clamp(0.6, 2.5));
        }
        let image = render.render(&mut scaler, key.glyph as u16)?;
        let p = image.placement;
        if p.width == 0 || p.height == 0 {
            return None;
        }
        Some(GlyphImage {
            width: p.width,
            height: p.height,
            left: p.left,
            top: -p.top,
            alpha: image.data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use biwa_player_core::geometry::Color;
    use biwa_player_core::text::TextStyle;

    fn span(text: &str, size: f32) -> TextSpan {
        TextSpan { text: text.into(), style: TextStyle { size, weight: 400, color: Color::WHITE, family: None } }
    }

    fn paragraph(width: f32) -> ParagraphStyle {
        ParagraphStyle { max_width: width, line_height: biwa_player_core::text::LineHeight::Relative(1.8), center: false }
    }

    #[test]
    fn maps_glyphs_back_to_spans_and_characters() {
        let mut t = TextSystem::new();
        let shaped = t.shape(&[span("こんにちは", 20.0), span("abc", 20.0)], paragraph(1000.0));
        assert!(!shaped.glyphs.is_empty(), "no glyphs: are system fonts installed?");
        let firsts: Vec<_> = shaped.glyphs.iter().map(|g| (g.span, g.char_index)).collect();
        assert_eq!(firsts.first(), Some(&(0, 0)));
        assert!(firsts.contains(&(0, 4)));
        assert!(firsts.contains(&(1, 2)));
    }

    /// 断片ごとに書式が違っても、同じ行の字は左から右へ重ならずに並ぶ。
    #[test]
    fn spans_on_one_line_do_not_overlap() {
        let mut t = TextSystem::new();
        let spans = [span("値: ", 20.0), span("5042", 20.0), span(" / ", 20.0), span("100", 20.0)];
        let shaped = t.shape(&spans, paragraph(1000.0));
        let xs: Vec<f32> = shaped.glyphs.iter().map(|g| g.x).collect();
        assert!(xs.windows(2).all(|w| w[0] < w[1]), "glyphs overlap: {xs:?}");
        assert_eq!(shaped.glyphs.len(), spans.iter().map(|s| s.text.chars().count()).sum::<usize>());
    }

    #[test]
    fn wraps_long_text_into_lines() {
        let mut t = TextSystem::new();
        let one = t.shape(&[span("あいうえおかきくけこ", 20.0)], paragraph(1000.0));
        let wrapped = t.shape(&[span("あいうえおかきくけこ", 20.0)], paragraph(70.0));
        assert!(wrapped.height > one.height * 2.0);
        let ys: std::collections::BTreeSet<_> = wrapped.glyphs.iter().map(|g| g.y as i32).collect();
        assert!(ys.len() > 2);
    }

    /// 行頭禁則の文字が行頭に来ないように、手前で折り返す。
    ///
    /// ICU は日本語のロケールのとき、小書きの仮名・長音符・句読点などの前ではもともと改行しない。
    /// 全角のチルダ「～」は ICU だけでは行頭に来るので、それで禁則の表 (`kinsoku.rs`) が効くことを見る。
    #[test]
    fn kinsoku_keeps_wave_dash_off_the_line_start() {
        let mut t = TextSystem::new();
        for c in ["～", "っ", "ー", "。"] {
            let text = format!("あいうえお{c}かきくけこさしすせそ");
            let one = t.shape(&[span(&text, 20.0)], paragraph(10_000.0));
            // 5 文字分の幅。禁則が無ければ 6 文字目が 2 行目の頭に来る。
            let width = one.glyphs[5].x - one.glyphs[0].x + 1.0;
            let shaped = t.shape(&[span(&text, 20.0)], paragraph(width));
            let line_start = shaped.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
            let glyph = shaped.glyphs.iter().find(|g| g.char_index == 5).unwrap();
            assert!(glyph.x > line_start + 1.0, "{c} starts a line");
        }
    }

    /// 太字を要求すると、普通の字とは別の鍵になり、太く描かれる
    /// (太字の無いフォントでも疑似ボールドで描き分ける)。
    #[test]
    fn bold_is_drawn_thicker() {
        let mut t = TextSystem::new();
        let mut bold = span("言", 28.0);
        bold.style.weight = 700;
        let shaped = t.shape(&[span("言", 28.0), bold], paragraph(1000.0));
        let (regular, bold) = (shaped.glyphs[0].key, shaped.glyphs[1].key);
        assert_ne!(regular, bold);
        let ink = |t: &mut TextSystem, key| -> u64 {
            t.rasterize(key).unwrap().alpha.iter().map(|a| u64::from(*a)).sum()
        };
        let (r, b) = (ink(&mut t, regular), ink(&mut t, bold));
        assert!(b > r * 11 / 10, "bold is not thicker: {b} vs {r}");
    }

    #[test]
    fn measures_width_and_centers_lines() {
        let mut t = TextSystem::new();
        let free = t.shape(&[span("あいう", 20.0)], paragraph(f32::INFINITY));
        assert!(free.width > 50.0 && free.width < 70.0, "width {}", free.width);
        let centered =
            t.shape(&[span("あいう", 20.0)], ParagraphStyle { center: true, ..paragraph(200.0) });
        let left = centered.glyphs.first().unwrap().x;
        assert!((left - (200.0 - free.width) / 2.0).abs() < 1.0, "left {left}");
    }

    #[test]
    fn rasterizes_a_glyph() {
        let mut t = TextSystem::new();
        let shaped = t.shape(&[span("あ", 32.0)], paragraph(1000.0));
        let image = t.rasterize(shaped.glyphs[0].key).expect("an image for あ");
        assert!(image.width > 10 && image.height > 10);
        assert_eq!(image.alpha.len(), (image.width * image.height) as usize);
        assert!(image.alpha.iter().any(|a| *a > 128));
    }
}
