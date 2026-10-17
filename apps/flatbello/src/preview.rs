//! The 2D preview: the selected faces' profiles, laid out exactly as they will be exported.

use egui::{Align2, Color32, FontId, Pos2, Sense, Shape, Stroke, Ui, Vec2};
use flatbello_core::{Bbox2, Point2, Profile2D, Provenance, Tolerance};

const EXACT: Color32 = Color32::from_rgb(220, 222, 228);
const DIMENSION: Color32 = Color32::from_rgb(150, 156, 168);

/// Colour of a fitted segment: green at no deviation, through yellow, to red at the fit
/// tolerance.
fn deviation_color(deviation: f64, fit: f64) -> Color32 {
    let t = (deviation / fit).clamp(0.0, 1.0) as f32;
    let (r, g) = if t < 0.5 {
        (t * 2.0, 1.0)
    } else {
        (1.0, 2.0 - t * 2.0)
    };
    Color32::from_rgb((r * 230.0) as u8, (g * 200.0) as u8, 60)
}

/// Draws `profiles` (already laid out) to fill the available space.
pub fn draw(ui: &mut Ui, profiles: &[Profile2D], tol: &Tolerance) {
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let painter = ui.painter_at(rect);
    let bbox = profiles
        .iter()
        .fold(Bbox2::EMPTY, |b, p| b.union(&p.bbox(tol)));
    if bbox.is_empty() {
        return;
    }
    let margin = 28.0;
    let inner = rect.shrink(margin);
    let scale = (f64::from(inner.width()) / bbox.width().max(1e-9))
        .min(f64::from(inner.height()) / bbox.height().max(1e-9));
    let center = Point2::new(
        0.5 * (bbox.min.x + bbox.max.x),
        0.5 * (bbox.min.y + bbox.max.y),
    );
    let to_screen = |p: Point2| {
        inner.center()
            + Vec2::new(
                ((p.x - center.x) * scale) as f32,
                (-(p.y - center.y) * scale) as f32,
            )
    };
    // Flatten to about half a pixel on screen.
    let flat = (0.5 / scale).min(tol.fit());

    for p in profiles {
        for l in std::iter::once(p.outer()).chain(p.holes()) {
            for e in l.edges() {
                let color = match e.provenance {
                    Provenance::Exact => EXACT,
                    Provenance::Fitted { max_deviation } => {
                        deviation_color(max_deviation, tol.fit())
                    }
                };
                let pts: Vec<Pos2> = e.segment.flatten(flat).into_iter().map(to_screen).collect();
                painter.add(Shape::line(pts, Stroke::new(1.5, color)));
            }
        }
        let b = p.bbox(tol);
        let label_at = to_screen(Point2::new(0.5 * (b.min.x + b.max.x), b.min.y));
        painter.text(
            label_at + Vec2::new(0.0, 6.0),
            Align2::CENTER_TOP,
            format!("{:.2} × {:.2} mm", b.width(), b.height()),
            FontId::proportional(12.0),
            DIMENSION,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deviation_colours_run_green_to_red() {
        let fit = 0.01;
        assert_eq!(deviation_color(0.0, fit), Color32::from_rgb(0, 200, 60));
        assert_eq!(deviation_color(fit, fit), Color32::from_rgb(230, 0, 60));
        assert_eq!(
            deviation_color(10.0 * fit, fit),
            Color32::from_rgb(230, 0, 60)
        );
    }
}
