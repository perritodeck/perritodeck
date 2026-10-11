//! Images from Stream Deck plugins (data URLs or files: PNG, JPEG, GIF, BMP, WebP, SVG),
//! turned into PNG for the deck.

use std::{
    path::Path,
    sync::{Arc, OnceLock},
};

use base64::{engine::general_purpose::STANDARD, Engine};

/// Size SVGs are drawn at (Stream Deck's high-resolution key size).
const SVG_PX: u32 = 144;
/// Plugin images bigger than this many bytes are ignored.
const MAX_BYTES: usize = 8 * 1024 * 1024;

/// A `setImage` value or manifest image as PNG bytes.
pub fn load(src: &str, folder: &Path) -> Option<Vec<u8>> {
    if let Some(rest) = src.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',')?;
        let bytes = if meta.ends_with(";base64") {
            STANDARD.decode(data.trim()).ok()?
        } else {
            percent_decode(data).into_bytes()
        };
        if bytes.len() > MAX_BYTES {
            return None;
        }
        let svg = meta.starts_with("image/svg");
        return to_png(&bytes, svg);
    }
    let file = crate::manifest::image_file(folder, src)?;
    let bytes = std::fs::read(&file).ok().filter(|b| b.len() <= MAX_BYTES)?;
    let svg = file
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    to_png(&bytes, svg)
}

/// PNG bytes from any supported image.
pub fn to_png(bytes: &[u8], svg: bool) -> Option<Vec<u8>> {
    if svg || looks_like_svg(bytes) {
        return svg_to_png(bytes);
    }
    if bytes.starts_with(b"\x89PNG") {
        return Some(bytes.to_vec());
    }
    let img = image::load_from_memory(bytes).ok()?;
    let mut png = Vec::new();
    img.to_rgba8()
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]);
    let head = head.trim_start();
    head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg"))
}

fn fonts() -> Arc<resvg::usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = resvg::usvg::fontdb::Database::new();
            db.load_font_data(perritodeck_render::FONT_BYTES.to_vec());
            db.load_system_fonts();
            // Generic names fall back to the bundled font when the system has none.
            db.set_sans_serif_family("Inter");
            Arc::new(db)
        })
        .clone()
}

fn svg_to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let opts = resvg::usvg::Options {
        fontdb: fonts(),
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_data(bytes, &opts).ok()?;
    let size = tree.size();
    let scale = SVG_PX as f32 / size.width().max(size.height()).max(1.0);
    let (w, h) = (
        (size.width() * scale).ceil().max(1.0) as u32,
        (size.height() * scale).ceil().max(1.0) as u32,
    );
    let mut pm = resvg::tiny_skia::Pixmap::new(w, h)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pm.as_mut(),
    );
    pm.encode_png().ok()
}

/// `%3Csvg%20…` → `<svg …`, as in `data:image/svg+xml;charset=utf8,…`.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_and_data_urls() {
        let dir = tempfile::tempdir().unwrap();
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="72" height="72"><rect width="72" height="72" fill="#ff0000"/><text x="10" y="40" font-family="sans-serif" font-size="20">7</text></svg>"##;
        let png = load(
            &format!(
                "data:image/svg+xml;charset=utf8,{}",
                svg.replace('<', "%3C").replace('>', "%3E")
            ),
            dir.path(),
        )
        .unwrap();
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!(img.width(), SVG_PX);
        assert_eq!(img.to_rgba8().get_pixel(1, 1).0, [255, 0, 0, 255]);

        let b64 = STANDARD.encode(svg);
        assert!(load(&format!("data:image/svg+xml;base64,{b64}"), dir.path()).is_some());
        assert!(load(
            &format!("data:image/png;base64,{}", STANDARD.encode(&png)),
            dir.path()
        )
        .is_some());
        std::fs::write(dir.path().join("key.svg"), svg).unwrap();
        assert!(
            load("key", dir.path()).is_some(),
            "manifest-style name without extension"
        );
        assert!(load("../key", dir.path()).is_none());
        assert!(load("data:image/png;base64,!!!", dir.path()).is_none());
        assert_eq!(percent_decode("a%20b%2"), "a b%2");
    }
}
