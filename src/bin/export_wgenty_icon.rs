use std::{fs, path::PathBuf};

use image::{ImageBuffer, Rgba};

use wgenty_code::cli::branding;

fn main() -> anyhow::Result<()> {
    let out_dir = PathBuf::from("assets/icons");
    fs::create_dir_all(&out_dir)?;

    fs::write(out_dir.join("wgenty-pixel.svg"), branding::icon_svg(16, 2))?;

    for size in [64_u32, 128, 256, 512] {
        write_png(&out_dir.join(format!("wgenty-pixel-{size}.png")), size)?;
    }

    write_png(&out_dir.join("wgenty-pixel.png"), 512)?;

    println!("exported wgenty icon assets to {}", out_dir.display());
    Ok(())
}

fn write_png(path: &PathBuf, size: u32) -> anyhow::Result<()> {
    let padding = size / 12;
    let drawable = size.saturating_sub(padding * 2).max(branding::ICON_WIDTH);
    let pixel = (drawable / branding::ICON_WIDTH).max(1);
    let actual = branding::ICON_WIDTH * pixel;
    let offset_x = (size.saturating_sub(actual)) / 2;
    let offset_y = (size.saturating_sub(branding::ICON_HEIGHT * pixel)) / 2;

    let mut img = ImageBuffer::from_pixel(size, size, Rgba([0, 0, 0, 0]));

    branding::for_each_icon_pixel(|x, y, rgba| {
        for dx in 0..pixel {
            for dy in 0..pixel {
                img.put_pixel(
                    offset_x + x * pixel + dx,
                    offset_y + y * pixel + dy,
                    Rgba(rgba),
                );
            }
        }
    });

    img.save(path)?;
    Ok(())
}
