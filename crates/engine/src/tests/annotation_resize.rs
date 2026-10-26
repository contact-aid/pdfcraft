//! Native review geometry: rendered ink, stable targets, undo/redo and approved removal.
use super::*;

fn review_shape(kind: &str) -> Shape {
    let quads = vec![pdfcraft_annot::rect_quad([20.0, 160.0, 100.0, 180.0])];
    match kind {
        "Highlight" => Shape::TextMarkup { kind: Markup::Highlight, quads },
        "StrikeOut" => Shape::TextMarkup { kind: Markup::StrikeOut, quads },
        "Redact" => Shape::Redact { quads, overlay: String::new(), look: Default::default() },
        "Text" => Shape::Note { at: [20.0, 160.0], icon: NoteIcon::Comment },
        _ => panic!("test shape"),
    }
}

fn changed_pixels(base: &pdfcraft_render::RenderedPage, marked: &pdfcraft_render::RenderedPage) -> usize {
    assert_eq!((base.width, base.height), (marked.width, marked.height));
    base.rgba.as_chunks::<4>().0.iter().zip(marked.rgba.as_chunks::<4>().0.iter()).filter(|(a, b)| a != b).count()
}

#[test]
fn review_resizes_render_and_keep_native_identity_through_history_and_save() {
    for rotation in [0, 90, 180, 270] {
        for kind in ["Highlight", "StrikeOut", "Redact", "Text"] {
            let (mut session, id) = session_with(1);
            if rotation != 0 {
                session.apply(id, Edit::RotatePages { pages: vec![0], degrees: rotation }).unwrap();
            }
            let unmarked = render_page(&session, id);
            let shape = review_shape(kind);
            session
                .apply(
                    id,
                    Edit::AddAnnotation(NewAnnotation {
                        page: 0,
                        style: Style::default_for(&shape),
                        shape,
                        contents: "Confirm annual maintenance renewal".into(),
                        author: "Facilities Procurement".into(),
                    }),
                )
                .unwrap();
            let original = session.get(id).unwrap().info.annotations.iter().find(|a| a.subtype == kind).unwrap().clone();
            let before = render_page(&session, id);
            let old_ink = changed_pixels(&unmarked, &before);
            assert!(old_ink > 0, "{kind} at rotation {rotation} must actually paint");
            let rect = [110.0, 40.0, 190.0, 100.0];
            session.apply(id, Edit::ResizeAnnotation { page: 0, index: original.index, rect }).unwrap();
            let resized = session.get(id).unwrap().info.annotations.iter().find(|a| a.name == original.name).unwrap().clone();
            assert_eq!(resized.index, original.index);
            assert_eq!(resized.rect, rect.map(|v| v as f32));
            assert_eq!(resized.author, original.author);
            assert_eq!(resized.contents, original.contents);
            let after = render_page(&session, id);
            assert_ne!(before.rgba, after.rgba, "{kind} must move its rendered appearance, not just metadata");
            if kind == "Text" {
                assert!(changed_pixels(&unmarked, &after) > old_ink * 3, "the resized note must paint a larger icon, including on rotated pages");
            } else {
                assert_ne!(resized.quads, original.quads);
            }
            session.undo(id).unwrap();
            assert_eq!(render_page(&session, id).rgba, before.rgba, "undo restores the old appearance");
            assert_eq!(session.get(id).unwrap().info.annotations.iter().find(|a| a.name == original.name).unwrap().rect, original.rect);
            session.redo(id).unwrap();
            assert_eq!(render_page(&session, id).rgba, after.rgba, "redo restores the resized appearance");
            let undo = session.get(id).unwrap().can_undo().map(str::to_owned);
            let bytes = session.save_bytes(id).unwrap();
            assert_eq!(session.get(id).unwrap().can_undo(), undo.as_deref(), "save never consumes live history");
            let mut reopened = Session::new();
            let saved_id = reopened.open("Maintenance Renewal Review.pdf", None, bytes, None).unwrap();
            let saved = reopened.get(saved_id).unwrap().info.annotations.iter().find(|a| a.name == original.name).unwrap();
            assert_eq!(saved.rect, resized.rect);
            assert_eq!(saved.quads, resized.quads);
            assert_eq!(render_page(&reopened, saved_id).rgba, after.rgba, "save/reopen retains the painted geometry");
        }
    }
}

fn shown_rect(session: &Session, id: DocId, needle: &str) -> [f64; 4] {
    let doc = session.get(id).unwrap();
    let mut renderer = pdfcraft_render::PageRenderer::new(doc.bytes.clone(), doc.config.clone());
    let text = renderer.render(RenderRequestFor::text(0)).text.unwrap();
    let range = text.find(needle).pop().unwrap();
    let rects = text.line_rects(range);
    assert_eq!(rects.len(), 1);
    doc.info.pages[0].view_rect_to_user(rects[0]).map(f64::from)
}

#[test]
fn resizing_redaction_changes_only_review_until_apply_then_removes_the_new_target() {
    let body = "BT /F1 16 Tf 20 200 Td (Renewal $4412.00) Tj 0 -60 Td (Public Maintenance Terms) Tj ET";
    let mut session = Session::new().with_clock(|| 1_700_000_000);
    let source = one_line_page("/Font << /F1 4 0 R >>", body);
    let id = session.open("Annual Maintenance Renewal.pdf", None, Arc::new(source), None).unwrap();
    let source_text = page_texts(&session, id);
    let target = shown_rect(&session, id, "Renewal $4412.00");
    mark_shown_text(&mut session, id, "Public Maintenance Terms");
    let mark = session.get(id).unwrap().info.annotations.iter().find(|a| a.subtype == "Redact").unwrap().clone();
    session.apply(id, Edit::ResizeAnnotation { page: 0, index: mark.index, rect: target }).unwrap();
    assert_eq!(page_texts(&session, id), source_text, "resize is review only");
    let new_mark = session.get(id).unwrap().info.annotations.iter().find(|a| a.name == mark.name).unwrap().clone();
    assert_ne!(new_mark.quads, mark.quads);
    session.undo(id).unwrap();
    assert_eq!(session.get(id).unwrap().info.annotations.iter().find(|a| a.name == mark.name).unwrap().quads, mark.quads);
    session.redo(id).unwrap();
    assert_eq!(session.get(id).unwrap().info.annotations.iter().find(|a| a.name == mark.name).unwrap().quads, new_mark.quads);
    session.apply(id, Edit::ApplyRedactions { pages: None }).unwrap();
    let removed = page_texts(&session, id).join("\n");
    assert!(!removed.contains("4412"), "the NEW resized review target must be removed: {removed}");
    assert!(removed.contains("Public Maintenance Terms"), "the OLD review area must be untouched: {removed}");
    assert_eq!(session.get(id).unwrap().redaction_marks(), 0);
    let bytes = session.save_bytes(id).unwrap();
    let mut reopened = Session::new();
    let saved_id = reopened.open("Approved Redacted Renewal.pdf", None, bytes, None).unwrap();
    assert_eq!(page_texts(&reopened, saved_id).join("\n"), removed);
    session.undo(id).unwrap();
    assert_eq!(page_texts(&session, id), source_text);
    assert_eq!(session.get(id).unwrap().redaction_marks(), 1);
}
