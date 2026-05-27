use egui::{pos2, vec2, Color32, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::cli::branding;

pub fn show(ui: &mut Ui, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    paint(ui, rect);
    response
}

pub fn paint(ui: &Ui, rect: Rect) {
    let painter = ui.painter();
    let pixel = (rect.width() / branding::ICON_WIDTH as f32)
        .min(rect.height() / branding::ICON_HEIGHT as f32)
        .floor()
        .max(1.0);
    let icon_size = vec2(
        branding::ICON_WIDTH as f32 * pixel,
        branding::ICON_HEIGHT as f32 * pixel,
    );
    let origin = pos2(
        rect.center().x - icon_size.x * 0.5,
        rect.center().y - icon_size.y * 0.5,
    );

    let shadow_rect = Rect::from_min_size(origin + vec2(pixel, pixel), icon_size);
    painter.rect_filled(
        shadow_rect,
        12.0,
        Color32::from_rgba_unmultiplied(115, 74, 169, 28),
    );

    branding::for_each_icon_pixel(|x, y, rgba| {
        let min = pos2(origin.x + x as f32 * pixel, origin.y + y as f32 * pixel);
        let cell = Rect::from_min_size(min, vec2(pixel, pixel));
        painter.rect_filled(
            cell,
            0.0,
            Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]),
        );
    });

    painter.rect_stroke(
        Rect::from_min_size(origin, icon_size),
        10.0,
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 10)),
        StrokeKind::Outside,
    );
}
