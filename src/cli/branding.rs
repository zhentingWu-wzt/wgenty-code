//! Shared pixel-branding assets for wgenty.

pub const ICON_WIDTH: u32 = 24;
pub const ICON_HEIGHT: u32 = 24;
const HALF_WIDTH: usize = 12;

const HALF_ROWS: [&str; ICON_HEIGHT as usize] = [
    "....1.......",
    "....11......",
    "...122......",
    "...1231.....",
    "..122231....",
    "..1233331...",
    ".123444331..",
    ".1231111331.",
    ".1231..11131",
    "12231...1131",
    "1231....1131",
    "1231.55.1131",
    "123155551131",
    "123122221131",
    "12312..21331",
    ".131....1231",
    ".131....1231",
    "..131..1231.",
    "..123111231.",
    "...1233321..",
    "...112221...",
    "....11211...",
    ".....111....",
    "......1.....",
];

pub fn for_each_icon_pixel(mut f: impl FnMut(u32, u32, [u8; 4])) {
    for (y, row) in HALF_ROWS.iter().enumerate() {
        debug_assert_eq!(row.len(), HALF_WIDTH);
        let bytes = row.as_bytes();

        for x in 0..HALF_WIDTH {
            if let Some(color) = palette(bytes[x]) {
                f(x as u32, y as u32, color);
            }
        }

        for x in 0..HALF_WIDTH {
            if let Some(color) = palette(bytes[HALF_WIDTH - 1 - x]) {
                f((HALF_WIDTH + x) as u32, y as u32, color);
            }
        }
    }
}

pub fn icon_svg(pixel_size: u32, padding: u32) -> String {
    let width = (ICON_WIDTH + padding * 2) * pixel_size;
    let height = (ICON_HEIGHT + padding * 2) * pixel_size;

    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width} {height}\" width=\"{width}\" height=\"{height}\">\n"
    ));
    svg.push_str("  <title>wgenty pixel icon</title>\n");
    svg.push_str("  <desc>Original pixel-art mech mask for wgenty.</desc>\n");
    svg.push_str("  <rect width=\"100%\" height=\"100%\" fill=\"none\"/>\n");

    for_each_icon_pixel(|x, y, rgba| {
        let px = (x + padding) * pixel_size;
        let py = (y + padding) * pixel_size;
        svg.push_str(&format!(
            "  <rect x=\"{px}\" y=\"{py}\" width=\"{pixel_size}\" height=\"{pixel_size}\" fill=\"#{:02x}{:02x}{:02x}\" fill-opacity=\"{:.3}\"/>\n",
            rgba[0],
            rgba[1],
            rgba[2],
            rgba[3] as f32 / 255.0
        ));
    });

    svg.push_str("</svg>\n");
    svg
}

fn palette(code: u8) -> Option<[u8; 4]> {
    match code {
        b'1' => Some([36, 17, 59, 255]),
        b'2' => Some([77, 43, 116, 255]),
        b'3' => Some([112, 74, 160, 255]),
        b'4' => Some([156, 127, 203, 255]),
        b'5' => Some([239, 233, 255, 255]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_dimensions_are_stable() {
        let mut max_x = 0;
        let mut max_y = 0;

        for_each_icon_pixel(|x, y, _| {
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        });

        assert!(max_x < ICON_WIDTH);
        assert!(max_y < ICON_HEIGHT);
    }
}
