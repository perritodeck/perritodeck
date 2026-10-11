//! Key images, drawn on the host at the tablet's real pixel size (like a Stream Deck),
//! so every client shows exactly the same thing and stays simple.
//!
//! Text uses the bundled Inter SemiBold (SIL OFL 1.1, see `assets/OFL.txt`).

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use base64::{engine::general_purpose::STANDARD, Engine};
use perritodeck_protocol::{Grid, Screen};
pub use tiny_skia::Pixmap;
use tiny_skia::{PixmapPaint, Transform};

/// The bundled Inter SemiBold, for anything else that draws text.
pub static FONT_BYTES: &[u8] = include_bytes!("../assets/Inter-SemiBold.otf");

/// Smallest and largest key image we render, in pixels.
pub const MIN_KEY_PX: u32 = 48;
pub const MAX_KEY_PX: u32 = 512;

#[derive(Debug, Clone, PartialEq)]
pub struct KeyStyle {
    pub title: String,
    pub background: [u8; 3],
    pub text: [u8; 3],
    /// PNG bytes drawn centred above the title (or filling the key when there's no title).
    pub icon_png: Option<Vec<u8>>,
}

/// Parse `#RRGGBB`.
pub fn parse_hex(color: &str) -> Option<[u8; 3]> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// Pixel size of one key on a device, mirroring the Android client's layout
/// (8dp outer padding, 8dp gaps, square keys fitted to the screen).
pub fn key_pixel_size(screen: &Screen, grid: Grid) -> u32 {
    let density = screen.density_dpi.max(1) as f32 / 160.0;
    let pad = 8.0 * density;
    let gap = 8.0 * density;
    let (cols, rows) = (grid.cols.max(1) as f32, grid.rows.max(1) as f32);
    let w = (screen.width_px as f32 - 2.0 * pad - gap * (cols - 1.0)) / cols;
    let h = (screen.height_px as f32 - 2.0 * pad - gap * (rows - 1.0)) / rows;
    (w.min(h).floor() as u32).clamp(MIN_KEY_PX, MAX_KEY_PX)
}

/// Render a square key image and return PNG bytes.
pub fn render_png(style: &KeyStyle, size: u32) -> Vec<u8> {
    render(style, size)
        .encode_png()
        .expect("encoding an in-memory pixmap")
}

/// `data:image/png;base64,…` for `setImage`.
pub fn data_url(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", STANDARD.encode(png))
}

pub fn render(style: &KeyStyle, size: u32) -> Pixmap {
    let icon = style
        .icon_png
        .as_deref()
        .and_then(|b| Pixmap::decode_png(b).ok());
    render_with_icon(style, icon.as_ref(), size)
}

/// Like [`render`], with the icon already decoded (one frame of an [`Animation`], say);
/// `style.icon_png` is ignored.
pub fn render_with_icon(style: &KeyStyle, icon: Option<&Pixmap>, size: u32) -> Pixmap {
    let size = size.clamp(MIN_KEY_PX, MAX_KEY_PX);
    let mut pm = Pixmap::new(size, size).expect("non-zero size");
    let [r, g, b] = style.background;
    pm.fill(tiny_skia::Color::from_rgba8(r, g, b, 255));

    let margin = size as f32 * 0.08;
    let inner = size as f32 - 2.0 * margin;
    let has_title = !style.title.trim().is_empty();

    // Icon: fills the key if there's no title, otherwise the top ~55%.
    let mut text_top = margin;
    if let Some(icon) = icon {
        let box_h = if has_title { inner * 0.55 } else { inner };
        let scale = (inner / icon.width() as f32).min(box_h / icon.height() as f32);
        let (w, h) = (icon.width() as f32 * scale, icon.height() as f32 * scale);
        let x = (size as f32 - w) / 2.0;
        let y = if has_title {
            margin
        } else {
            (size as f32 - h) / 2.0
        };
        pm.draw_pixmap(
            0,
            0,
            icon.as_ref(),
            &PixmapPaint {
                quality: tiny_skia::FilterQuality::Bicubic,
                ..Default::default()
            },
            Transform::from_scale(scale, scale).post_translate(x, y),
            None,
        );
        text_top = y + h + margin * 0.5;
    }

    if has_title {
        let area_h = size as f32 - margin - text_top;
        draw_title(
            &mut pm,
            &style.title,
            style.text,
            margin,
            text_top,
            inner,
            area_h,
        );
    }
    pm
}

/// A key whose image fills it edge to edge (as a Stream Deck plugin draws it), with the
/// title on top, at the bottom, outlined so it reads on any image.
pub fn render_full_image(image: &Pixmap, title: &str, text: [u8; 3], size: u32) -> Pixmap {
    let size = size.clamp(MIN_KEY_PX, MAX_KEY_PX);
    let mut pm = Pixmap::new(size, size).expect("non-zero size");
    pm.fill(tiny_skia::Color::BLACK);
    let scale = (size as f32 / image.width() as f32).max(size as f32 / image.height() as f32);
    let (w, h) = (image.width() as f32 * scale, image.height() as f32 * scale);
    pm.draw_pixmap(
        0,
        0,
        image.as_ref(),
        &PixmapPaint {
            quality: tiny_skia::FilterQuality::Bicubic,
            ..Default::default()
        },
        Transform::from_scale(scale, scale)
            .post_translate((size as f32 - w) / 2.0, (size as f32 - h) / 2.0),
        None,
    );
    if !title.trim().is_empty() {
        let margin = size as f32 * 0.06;
        let inner = size as f32 - 2.0 * margin;
        let (top, area) = (size as f32 * 0.55, size as f32 * 0.45 - margin);
        // A dark outline first, then the text.
        let outline = [0, 0, 0];
        let d = (size as f32 / 72.0).max(1.0);
        for (dx, dy) in [(-d, 0.0), (d, 0.0), (0.0, -d), (0.0, d)] {
            draw_title(&mut pm, title, outline, margin + dx, top + dy, inner, area);
        }
        draw_title(&mut pm, title, text, margin, top, inner, area);
    }
    pm
}

/// Lay out `title` (explicit `\n` breaks plus word wrap), shrinking the font until it
/// fits the box, then draw it centred.
fn draw_title(pm: &mut Pixmap, title: &str, color: [u8; 3], x0: f32, y0: f32, w: f32, h: f32) {
    let font = FontRef::try_from_slice(FONT_BYTES).expect("bundled font is valid");
    let mut px = (w * 0.24).max(8.0);
    let (lines, scaled) = loop {
        let scaled = font.as_scaled(PxScale::from(px));
        let lines = wrap(title, &scaled, w);
        let height = lines.len() as f32 * line_height(&scaled);
        let widest = lines
            .iter()
            .map(|l| text_width(l, &scaled))
            .fold(0.0, f32::max);
        if (height <= h && widest <= w) || px <= 8.0 {
            break (lines, scaled);
        }
        px *= 0.92;
    };

    let lh = line_height(&scaled);
    let block = lines.len() as f32 * lh;
    let mut baseline = y0 + (h - block).max(0.0) / 2.0 + scaled.ascent();
    let (width, height) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    for line in &lines {
        let lw = text_width(line, &scaled);
        let mut caret = x0 + (w - lw).max(0.0) / 2.0;
        let mut prev = None;
        for ch in line.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(p) = prev {
                caret += scaled.kern(p, id);
            }
            let glyph =
                id.with_scale_and_position(scaled.scale(), ab_glyph::point(caret, baseline));
            caret += scaled.h_advance(id);
            prev = Some(id);
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|gx, gy, coverage| {
                    let px = bounds.min.x as i32 + gx as i32;
                    let py = bounds.min.y as i32 + gy as i32;
                    if px < 0 || py < 0 || px >= width || py >= height {
                        return;
                    }
                    let i = ((py * width + px) * 4) as usize;
                    let a = coverage.clamp(0.0, 1.0);
                    for (c, &src) in color.iter().enumerate() {
                        let dst = data[i + c] as f32;
                        data[i + c] = (dst + (src as f32 - dst) * a).round() as u8;
                    }
                });
            }
        }
        baseline += lh;
    }
}

fn line_height<F: Font>(f: &ab_glyph::PxScaleFont<F>) -> f32 {
    (f.ascent() - f.descent() + f.line_gap()) * 1.05
}

fn text_width<F: Font>(s: &str, f: &ab_glyph::PxScaleFont<F>) -> f32 {
    let mut w = 0.0;
    let mut prev = None;
    for ch in s.chars() {
        let id = f.glyph_id(ch);
        if let Some(p) = prev {
            w += f.kern(p, id);
        }
        w += f.h_advance(id);
        prev = Some(id);
    }
    w
}

fn wrap<F: Font>(text: &str, f: &ab_glyph::PxScaleFont<F>, max_w: f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if text_width(&candidate, f) <= max_w || line.is_empty() {
                line = candidate;
            } else {
                out.push(std::mem::take(&mut line));
                line = word.to_owned();
            }
        }
        out.push(line);
    }
    out
}

/// An animated icon: its frames, already composited, and how long each one shows.
#[derive(Debug, Clone)]
pub struct Animation {
    pub frames: Vec<Pixmap>,
    pub delays_ms: Vec<u32>,
}

/// Animations are capped so a huge GIF can't eat memory or flood a tablet.
pub const MAX_FRAMES: usize = 120;
/// Shortest time a frame shows: about 12 frames a second, kind to Wi-Fi and old tablets.
pub const MIN_FRAME_MS: u32 = 80;
const MAX_ICON_SIDE: u32 = 288;

impl Animation {
    /// One run through all frames.
    pub fn duration_ms(&self) -> u64 {
        self.delays_ms.iter().map(|&d| u64::from(d)).sum()
    }

    /// Which frame shows `elapsed_ms` after the animation started (it loops).
    pub fn frame_at(&self, elapsed_ms: u64) -> usize {
        let total = self.duration_ms().max(1);
        let mut t = elapsed_ms % total;
        for (i, &d) in self.delays_ms.iter().enumerate() {
            if t < u64::from(d) {
                return i;
            }
            t -= u64::from(d);
        }
        0
    }
}

/// Whether these bytes are a GIF (animated or not).
pub fn is_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
}

/// Decode an animated GIF. `None` if it isn't a GIF, can't be read, or has only one
/// frame (a still image is drawn like any other icon).
pub fn decode_animation(bytes: &[u8]) -> Option<Animation> {
    use image::AnimationDecoder;
    if !is_gif(bytes) {
        return None;
    }
    let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    let mut frames = Vec::new();
    let mut delays_ms = Vec::new();
    for frame in decoder.into_frames().take(MAX_FRAMES) {
        let frame = frame.ok()?;
        let (num, den) = frame.delay().numer_denom_ms();
        let delay = num.checked_div(den).unwrap_or(100);
        // Browsers treat 0–10 ms as "as fast as possible" and show ~100 ms; do the same.
        let delay = if delay <= 10 { 100 } else { delay };
        let mut rgba = frame.into_buffer();
        if rgba.width() > MAX_ICON_SIDE || rgba.height() > MAX_ICON_SIDE {
            rgba = image::imageops::thumbnail(&rgba, MAX_ICON_SIDE, MAX_ICON_SIDE);
        }
        frames.push(pixmap_from_rgba(&rgba)?);
        delays_ms.push(delay.max(MIN_FRAME_MS));
    }
    (frames.len() > 1).then_some(Animation { frames, delays_ms })
}

/// The first frame of a GIF, for a still image.
pub fn decode_still(bytes: &[u8]) -> Option<Pixmap> {
    if !is_gif(bytes) {
        return Pixmap::decode_png(bytes).ok();
    }
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Gif).ok()?;
    pixmap_from_rgba(&img.to_rgba8())
}

fn pixmap_from_rgba(rgba: &image::RgbaImage) -> Option<Pixmap> {
    let mut pm = Pixmap::new(rgba.width(), rgba.height())?;
    // tiny-skia wants premultiplied alpha.
    for (dst, src) in pm.pixels_mut().iter_mut().zip(rgba.pixels()) {
        let [r, g, b, a] = src.0;
        *dst = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
    }
    Some(pm)
}

/// A small two-frame GIF (red, then blue), for tests in other crates.
#[doc(hidden)]
pub fn test_gif() -> Vec<u8> {
    use image::{codecs::gif::GifEncoder, Delay, Frame, Rgba, RgbaImage};
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        for (colour, ms) in [([255, 0, 0, 255], 30), ([0, 0, 255, 255], 500)] {
            let img = RgbaImage::from_pixel(16, 16, Rgba(colour));
            enc.encode_frame(Frame::from_parts(
                img,
                0,
                0,
                Delay::from_numer_denom_ms(ms, 1),
            ))
            .expect("encoding a GIF in memory");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(title: &str) -> KeyStyle {
        KeyStyle {
            title: title.into(),
            background: [0x1E, 0x1E, 0x2E],
            text: [0xFF, 0xFF, 0xFF],
            icon_png: None,
        }
    }

    fn rgb(pm: &Pixmap, x: u32, y: u32) -> [u8; 3] {
        let p = pm.pixel(x, y).unwrap().demultiply();
        [p.red(), p.green(), p.blue()]
    }

    /// Fraction of pixels that are noticeably lighter than the background.
    fn ink(pm: &Pixmap) -> f32 {
        let lit = pm.pixels().iter().filter(|p| p.red() > 0x80).count();
        lit as f32 / pm.pixels().len() as f32
    }

    #[test]
    fn gifs_animate() {
        let gif = test_gif();
        assert!(is_gif(&gif));
        let anim = decode_animation(&gif).expect("two frames");
        assert_eq!(anim.frames.len(), 2);
        assert_eq!(
            anim.delays_ms,
            vec![MIN_FRAME_MS, 500],
            "fast frames are slowed down"
        );
        assert_eq!(anim.frame_at(0), 0);
        assert_eq!(anim.frame_at(u64::from(MIN_FRAME_MS)), 1);
        assert_eq!(anim.frame_at(anim.duration_ms() + 1), 0, "it loops");
        let px = anim.frames[1].pixel(8, 8).unwrap().demultiply();
        assert_eq!([px.red(), px.green(), px.blue()], [0, 0, 255]);

        let still = decode_still(&gif).expect("first frame");
        let key = render_with_icon(&style(""), Some(&still), 96);
        assert_eq!(
            rgb(&key, 48, 48),
            [255, 0, 0],
            "the icon fills a key with no title"
        );
        assert!(decode_animation(b"not a gif").is_none());
    }

    #[test]
    fn background_and_text_are_drawn() {
        let pm = render(&style("Mute"), 144);
        assert_eq!((pm.width(), pm.height()), (144, 144));
        assert_eq!(
            rgb(&pm, 0, 0),
            [0x1E, 0x1E, 0x2E],
            "corners keep the background"
        );
        let ink = ink(&pm);
        assert!(
            ink > 0.01 && ink < 0.4,
            "some, but not too much, text ink: {ink}"
        );
    }

    #[test]
    fn empty_title_draws_only_background() {
        assert_eq!(ink(&render(&style("  "), 96)), 0.0);
    }

    #[test]
    fn long_titles_shrink_to_fit_inside_the_margins() {
        let pm = render(
            &style("Supercalifragilistic expialidocious scene number twelve"),
            128,
        );
        let margin = (128.0 * 0.08) as u32 - 1;
        for y in 0..128 {
            for x in (0..margin).chain(128 - margin..128) {
                assert_eq!(
                    rgb(&pm, x, y),
                    [0x1E, 0x1E, 0x2E],
                    "ink at ({x},{y}) is outside the margin"
                );
            }
        }
        assert!(ink(&pm) > 0.0);
    }

    #[test]
    fn icons_are_drawn() {
        let mut icon = Pixmap::new(10, 10).unwrap();
        icon.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        let s = KeyStyle {
            icon_png: Some(icon.encode_png().unwrap()),
            ..style("")
        };
        let pm = render(&s, 100);
        assert_eq!(
            rgb(&pm, 50, 50),
            [255, 0, 0],
            "icon fills the middle when there's no title"
        );
        assert_eq!(rgb(&pm, 1, 1), [0x1E, 0x1E, 0x2E]);
    }

    #[test]
    fn bad_icon_bytes_are_ignored() {
        let s = KeyStyle {
            icon_png: Some(b"not a png".to_vec()),
            ..style("A")
        };
        assert_eq!(render(&s, 64).width(), 64);
    }

    #[test]
    fn png_and_data_url() {
        let png = render_png(&style("Go\nLive"), 72);
        assert_eq!(&png[1..4], b"PNG");
        assert!(data_url(&png).starts_with("data:image/png;base64,iVBOR"));
    }

    #[test]
    fn key_size_matches_client_layout() {
        // Fire HD 10: 1920×1200 @ 224 dpi, 8×5 grid → (1200 − 6 × 11.2 px) ÷ 5 ≈ 226 px.
        let s = Screen {
            width_px: 1920,
            height_px: 1200,
            density_dpi: 224,
        };
        let px = key_pixel_size(&s, Grid { cols: 8, rows: 5 });
        assert_eq!(px, 226);
        assert_eq!(
            key_pixel_size(
                &Screen {
                    width_px: 10,
                    height_px: 10,
                    density_dpi: 0
                },
                Grid { cols: 9, rows: 9 }
            ),
            MIN_KEY_PX
        );
    }

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex("#1E1E2E"), Some([0x1E, 0x1E, 0x2E]));
        assert_eq!(parse_hex("1E1E2E"), None);
        assert_eq!(parse_hex("#XYZ123"), None);
    }
}

#[cfg(test)]
mod preview {
    /// `PERRITODECK_RENDER_PREVIEW=dir cargo test -p perritodeck-render preview` writes sample keys for eyeballing.
    #[test]
    fn write_preview() {
        let Some(dir) = std::env::var_os("PERRITODECK_RENDER_PREVIEW") else {
            return;
        };
        let samples = [
            ("mute", "Mic\nmuted", [0xF3, 0x8B, 0xA8], [0x11, 0x11, 0x1B]),
            ("clock", "21:07:45", [0x1E, 0x1E, 0x2E], [0xCD, 0xD6, 0xF4]),
            (
                "scene",
                "Scene: Just Chatting",
                [0x89, 0xB4, 0xFA],
                [0x11, 0x11, 0x1B],
            ),
            (
                "long",
                "Supercalifragilistic expialidocious",
                [0xA6, 0xE3, 0xA1],
                [0x11, 0x11, 0x1B],
            ),
        ];
        for (name, title, bg, fg) in samples {
            let s = super::KeyStyle {
                title: title.into(),
                background: bg,
                text: fg,
                icon_png: None,
            };
            std::fs::write(
                std::path::Path::new(&dir).join(format!("{name}.png")),
                super::render_png(&s, 226),
            )
            .unwrap();
        }
    }
}
