//! Controls belonging to the inline editor overlay, above its text input. They use their own
//! widget responses, so typing cannot swallow a frame handle's drag.
use super::{ACCENT, LineEditor, input_id};
use crate::canvas::PageXform;
use egui::{CornerRadius, Pos2, Rect, Stroke, pos2};
use pdfcraft_engine::{Edit, ObjectKind, ObjectTarget};
use pdfcraft_render::DocInfo;

#[derive(Clone, Copy)]
struct Gesture {
    grip: usize,
    start: Pos2,
    now: Pos2,
    rect: Rect,
    cancelled: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn controls(
    ui: &mut egui::Ui,
    ed: &LineEditor,
    xf: &PageXform,
    info: &DocInfo,
    rect: Rect,
    scale: f32,
    doc: pdfcraft_engine::DocId,
) -> (bool, Option<Edit>, bool) {
    if ed.turned
        || !scale.is_finite()
        || scale <= 0.0
        || !rect.is_positive()
        || ![rect.left(), rect.top(), rect.right(), rect.bottom()].into_iter().all(f32::is_finite)
    {
        return (false, None, false);
    }
    let id = input_id().with(("frame-gesture", doc, ed.page, ed.block));
    let mut gesture = ui.ctx().data(|d| d.get_temp::<Gesture>(id));
    if ui.input(|i| !i.focused || i.key_pressed(egui::Key::Escape))
        && let Some(g) = gesture.as_mut()
    {
        g.cancelled = true;
    }
    let clip = ui.clip_rect();
    let visible = rect.intersect(clip);
    if !visible.is_positive() {
        return (false, None, false);
    }
    let height = visible.height().clamp(8.0, 24.0);
    let left = if rect.left() - 10.0 >= clip.left() { rect.left() - 6.0 } else { rect.left() + 4.0 };
    let right = if rect.right() + 10.0 <= clip.right() { rect.right() + 6.0 } else { rect.right() - 4.0 };
    let controls = [
        (Rect::from_center_size(pos2(left, visible.center().y), egui::vec2(8.0, height)), tl!("Resize paragraph from left")),
        (Rect::from_center_size(pos2(right, visible.center().y), egui::vec2(8.0, height)), tl!("Resize paragraph from right")),
        (
            Rect::from_center_size(pos2(rect.center().x, rect.top() - 6.5), egui::vec2((rect.width() - 16.0).clamp(1.0, 32.0), 7.0)),
            tl!("Move paragraph"),
        ),
    ];
    let mut held = false;
    let mut selected = false;
    let mut edit = None;
    for (grip, (area, label)) in controls.into_iter().enumerate() {
        let area = area.intersect(ui.clip_rect());
        if !area.is_positive() {
            continue;
        }
        let clip = ui.clip_rect();
        let response = egui::Area::new(id.with(grip))
            .order(egui::Order::Foreground)
            .fixed_pos(area.min)
            .constrain(false)
            .movable(false)
            .fade_in(false)
            .show(ui.ctx(), |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(clip));
                let (rect, response) = ui.allocate_exact_size(area.size(), egui::Sense::click_and_drag());
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
                let cursor = if grip == 2 { egui::CursorIcon::Move } else { egui::CursorIcon::ResizeHorizontal };
                response.clone().on_hover_cursor(cursor).on_hover_text(label);
                ui.painter().rect_filled(rect, CornerRadius::same(2), ACCENT);
                response
            })
            .inner;
        if response.clicked() && !ed.has_changes() {
            selected = true;
            held = true;
        }
        if gesture.is_none()
            && response.is_pointer_button_down_on()
            && let Some(start) = response.interact_pointer_pos().filter(|p| p.x.is_finite() && p.y.is_finite())
        {
            gesture = Some(Gesture { grip, start, now: start, rect, cancelled: false });
        }
        let Some(g) = gesture.as_mut().filter(|g| g.grip == grip) else { continue };
        held = true;
        if let Some(now) = response.interact_pointer_pos().filter(|p| p.x.is_finite() && p.y.is_finite()) {
            g.now = now;
        }
        let delta = g.now - g.start;
        let mut preview = g.rect;
        match grip {
            0 => preview.min.x = (preview.left() + delta.x).min(preview.right() - 12.0 * scale).max(xf.rect.left()),
            1 => preview.max.x = (preview.right() + delta.x).max(preview.left() + 12.0 * scale).min(xf.rect.right()),
            _ => preview = preview.translate(delta),
        }
        if g.cancelled {
            preview = g.rect;
        }
        ui.painter().rect_stroke(preview.expand(2.0), CornerRadius::ZERO, Stroke::new(1.5, ACCENT), egui::StrokeKind::Outside);
        if response.drag_stopped() {
            if !g.cancelled
                && preview != g.rect
                && let Some(page) = info.pages.get(ed.page)
            {
                let start = xf.screen_to_view(g.rect.min);
                let end = xf.screen_to_view(preview.min);
                let a = page.view_to_user(start.0, start.1);
                let b = page.view_to_user(end.0, end.1);
                let offset = [f64::from(b[0] - a[0]), f64::from(b[1] - a[1])];
                let width = f64::from(preview.width() / scale);
                if offset.into_iter().all(f64::is_finite) && width.is_finite() {
                    let mut style = ed.style();
                    if grip < 2 {
                        style.width = Some(width);
                    }
                    if offset != [0.0, 0.0] {
                        style.offset = Some(offset);
                    }
                    selected = grip == 2;
                    edit = Some(if grip == 2 && !ed.has_changes() {
                        Edit::MoveObjects { page: ed.page, objects: vec![ObjectTarget { kind: ObjectKind::Text, index: ed.block }], offset }
                    } else {
                        let text = if grip < 2 && !ed.has_changes() {
                            ed.source.as_ref().map_or_else(|| ed.text.clone(), |(b, _)| b.reflow_text.clone())
                        } else {
                            ed.text.clone()
                        };
                        Edit::EditTextBlock { page: ed.page, block: ed.block, text, style }
                    });
                }
            }
            gesture = None;
        } else if !response.is_pointer_button_down_on() && !response.dragged() {
            gesture = None;
        }
    }
    ui.ctx().data_mut(|d| {
        if let Some(g) = gesture {
            d.insert_temp(id, g);
        } else {
            d.remove::<Gesture>(id);
        }
    });
    (held, edit, selected)
}
