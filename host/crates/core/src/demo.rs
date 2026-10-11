//! The Phase 1 demo layout: a grid sized to the device, a live clock key and a
//! press counter, so both directions of the protocol are visible on screen.

use perritodeck_protocol::{Grid, Hello, KeySpec, Layout};

pub const CLOCK_KEY: u32 = 0;
pub const COUNTER_KEY: u32 = 1;

const PALETTE: &[(&str, &str)] = &[
    ("#1E1E2E", "#CDD6F4"),
    ("#F38BA8", "#11111B"),
    ("#FAB387", "#11111B"),
    ("#F9E2AF", "#11111B"),
    ("#A6E3A1", "#11111B"),
    ("#94E2D5", "#11111B"),
    ("#89B4FA", "#11111B"),
    ("#CBA6F7", "#11111B"),
];

/// Pick a grid for a device: its request if valid, otherwise from its screen
/// size — 5×3 on phones, 8×5 on tablets, flipped for portrait.
pub fn grid_for(hello: &Hello) -> Grid {
    if let Some(g) = hello.requested_grid.filter(Grid::is_valid) {
        return g;
    }
    let s = hello.screen;
    let dpi = s.density_dpi.max(1);
    // Android's own tablet threshold: smallest width ≥ 600dp.
    let smallest_width_dp = s.width_px.min(s.height_px) * 160 / dpi;
    let (long, short) = if smallest_width_dp >= 600 {
        (8, 5)
    } else {
        (5, 3)
    };
    if s.width_px >= s.height_px {
        Grid {
            cols: long,
            rows: short,
        }
    } else {
        Grid {
            cols: short,
            rows: long,
        }
    }
}

pub fn layout_for(hello: &Hello) -> Layout {
    layout_for_grid(grid_for(hello))
}

pub fn layout_for_grid(grid: Grid) -> Layout {
    let keys = (0..grid.key_count())
        .map(|index| {
            let (bg, fg) = PALETTE[index as usize % PALETTE.len()];
            let title = match index {
                CLOCK_KEY => "--:--:--".to_owned(),
                COUNTER_KEY => counter_title(0),
                n => format!("Key {}", n + 1),
            };
            KeySpec {
                index,
                title,
                bg_color: bg.to_owned(),
                text_color: fg.to_owned(),
                control: None,
                value: None,
            }
        })
        .collect();
    Layout {
        cols: grid.cols,
        rows: grid.rows,
        keys,
    }
}

pub fn counter_title(n: u64) -> String {
    format!("Pressed\n{n}×")
}

#[cfg(test)]
mod tests {
    use super::*;
    use perritodeck_protocol::{Platform, Screen};

    fn hello(w: u32, h: u32, dpi: u32, req: Option<Grid>) -> Hello {
        Hello {
            device_id: "d".into(),
            device_name: "n".into(),
            app_version: "0".into(),
            platform: Platform::Android,
            os_version: "9".into(),
            screen: Screen {
                width_px: w,
                height_px: h,
                density_dpi: dpi,
            },
            capabilities: vec![],
            requested_grid: req,
            token: None,
            device_secret: None,
        }
    }

    #[test]
    fn tablet_landscape_and_portrait() {
        assert_eq!(
            grid_for(&hello(1920, 1200, 224, None)),
            Grid { cols: 8, rows: 5 }
        );
        assert_eq!(
            grid_for(&hello(1200, 1920, 224, None)),
            Grid { cols: 5, rows: 8 }
        );
    }

    #[test]
    fn phone_gets_small_grid() {
        assert_eq!(
            grid_for(&hello(1080, 2400, 420, None)),
            Grid { cols: 3, rows: 5 }
        );
    }

    #[test]
    fn valid_request_wins_invalid_ignored() {
        let g = Grid { cols: 4, rows: 2 };
        assert_eq!(grid_for(&hello(1080, 2400, 420, Some(g))), g);
        let bad = Grid { cols: 0, rows: 99 };
        assert_eq!(
            grid_for(&hello(1080, 2400, 420, Some(bad))),
            Grid { cols: 3, rows: 5 }
        );
    }

    #[test]
    fn layout_has_one_key_per_cell_with_valid_colors() {
        let l = layout_for(&hello(1920, 1200, 224, None));
        assert_eq!(l.keys.len(), 40);
        assert!(l.keys.iter().enumerate().all(|(i, k)| k.index == i as u32));
        assert!(l
            .keys
            .iter()
            .all(|k| perritodeck_protocol::is_color(&k.bg_color)
                && perritodeck_protocol::is_color(&k.text_color)));
    }

    #[test]
    fn zero_dpi_does_not_panic() {
        let _ = grid_for(&hello(800, 600, 0, None));
    }
}
