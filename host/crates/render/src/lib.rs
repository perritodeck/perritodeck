//! Key images, drawn on the host at the tablet's real pixel size (like a Stream Deck),
//! so every client shows exactly the same thing and stays simple.
//!
//! Text uses the bundled Inter SemiBold (SIL OFL 1.1, see `assets/OFL.txt`).

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use base64::{engine::general_purpose::STANDARD, Engine};
use perritodeck_protocol::{Grid, Screen};
use tiny_skia::{Pixmap, PixmapPaint, Transform};

static FONT_BYTES: &[u8] = include_bytes!("../assets/Inter-SemiBold.otf");

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
    let size = size.clamp(MIN_KEY_PX, MAX_KEY_PX);
    let mut pm = Pixmap::new(size, size).expect("non-zero size");
    let [r, g, b] = style.background;
    pm.fill(tiny_skia::Color::from_rgba8(r, g, b, 255));

    let margin = size as f32 * 0.08;
    let inner = size as f32 - 2.0 * margin;
    let has_title = !style.title.trim().is_empty();

    // Icon: fills the key if there's no title, otherwise the top ~55%.
    let mut text_top = margin;
    if let Some(icon) = style
        .icon_png
        .as_deref()
        .and_then(|b| Pixmap::decode_png(b).ok())
    {
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
