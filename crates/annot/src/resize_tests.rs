//! Focused tests for `set_rect` on Highlight, StrikeOut, Redact and Text (note) comments:
//! native geometry (`/QuadPoints`) scaling, identity/metadata preservation, appearance
//! behaviour and refusal of hostile or locked input. Nothing here is shared with `tests.rs`.

use std::sync::Arc;

use pdfcraft_cos::{Document, ObjRef, Object, SaveOptions, write_incremental};

use super::*;

/// One page, no annotations; comments are added through the builders.
fn doc() -> Document {
    let objs: Vec<&[u8]> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>",                                 // 1
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 612 792] >>", // 2
        b"<< /Type /Page /Parent 2 0 R >>",                                   // 3
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    Document::open(Arc::new(out)).expect("opens")
}

fn meta(id: &str) -> Meta {
    Meta { date: Some("D:20261001120000Z".into()), id: id.into() }
}

fn new(page: usize, shape: Shape) -> NewAnnotation {
    NewAnnotation {
        page,
        shape,
        style: Style { color: [0.0, 0.4, 1.0], opacity: 1.0, width: 2.0, fill: None },
        contents: "Review renewal limits\nConfirm service coverage".into(),
        author: "Ada".into(),
    }
}

/// Save incrementally and reopen, so every check runs against what a reader would see.
fn reopen(doc: &Document) -> Document {
    let bytes = write_incremental(doc, &SaveOptions::default()).expect("writes");
    hayro_syntax::Pdf::new(bytes.clone()).expect("hayro-syntax parses the output");
    Document::open(Arc::new(bytes)).expect("reopens")
}

fn annot(doc: &Document, page: usize, index: usize) -> (ObjRef, Dict) {
    let p = page_refs(doc).unwrap()[page];
    let r = annots(doc, p)[index].as_ref().expect("indirect annotation");
    (r, annot_dict(doc, r))
}

fn nums(d: &Dict, key: &[u8]) -> Vec<f64> {
    d.get(key).unwrap().as_array().unwrap().iter().map(|o| o.as_f64().unwrap()).collect()
}

fn snap(doc: &Document, r: ObjRef) -> String {
    // Debug formatting, not PartialEq: /QuadPoints may hold NaN, which is never equal to itself.
    format!("{:?}", doc.get(r))
}

fn ap_n(doc: &Document, d: &Dict) -> ObjRef {
    d.get(b"AP").map(|a| doc.resolve(a)).and_then(|a| a.as_dict().and_then(|a| a.reference(b"N"))).expect("has /AP /N")
}

fn ap_content(doc: &Document, d: &Dict) -> String {
    let obj = doc.get(ap_n(doc, d));
    let Object::Stream(s) = &*obj else { panic!("AP is not a stream") };
    String::from_utf8_lossy(&s.decoded().unwrap()).into_owned()
}

/// A review scenario: a two-line highlight whose first line is oblique (a scanned, slightly
/// skewed page), with a reply and an unknown foreign key, exactly as another reviewer leaves it.
fn reviewed_highlight(doc: &mut Document) -> usize {
    let quads = vec![
        [72.0, 712.0, 200.0, 710.0, 72.0, 706.0, 200.0, 704.0], // oblique first line
        [72.0, 706.0, 200.0, 706.0, 72.0, 700.0, 200.0, 700.0], // second line
    ];
    let h = add_annotation(doc, &new(0, Shape::TextMarkup { kind: Markup::Highlight, quads }), &meta("quote-1")).unwrap();
    let (_, d) = annot(doc, 0, h);
    assert_eq!(nums(&d, b"Rect"), [72.0, 700.0, 200.0, 712.0], "the builder's /Rect is the tight quad bounds");
    add_reply(doc, 0, h, "Which clause is this?", "Bob", &meta("")).unwrap();
    let (r, _) = annot(doc, 0, h);
    doc.update_dict(r, |d| d.set(b"Vendor".to_vec(), Object::name("Acme"))).unwrap();
    h
}

/// `t` maps the old `/Rect` onto the new one; check every scaled value within f64 noise.
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}

#[test]
fn resize_highlight_scales_every_quad_and_keeps_identity() {
    let mut doc = doc();
    let h = reviewed_highlight(&mut doc);
    set_rect(&mut doc, 0, h, [100.0, 200.0, 300.0, 424.0], &Meta { date: Some("D:20261002143000Z".into()), id: "not-a-new-comment".into() }).unwrap();
    let doc = reopen(&doc);
    let (_, d) = annot(&doc, 0, h);
    assert_eq!(d.name(b"Subtype"), Some(&b"Highlight"[..]));
    assert_eq!(nums(&d, b"Rect"), [100.0, 200.0, 300.0, 424.0]);
    // Both quads survive, in source order, scaled with the same map from the old rectangle.
    let q = nums(&d, b"QuadPoints");
    assert_eq!(q.len(), 16, "the two lines must stay two quads");
    // x: [72, 200] -> [100, 300]; y: [700, 712] -> [200, 424].
    for (i, want) in [100.0, 424.0, 300.0, 386.6666666666667, 100.0, 312.0, 300.0, 274.6666666666667].iter().enumerate() {
        near(q[i], *want);
    }
    for (i, want) in [100.0, 312.0, 300.0, 312.0, 100.0, 200.0, 300.0, 200.0].iter().enumerate() {
        near(q[8 + i], *want);
    }
    // Tight bounds and /Rect agree after the scale.
    let (pairs, _) = q.as_chunks::<2>();
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in pairs {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    near(lo[0], 100.0);
    near(lo[1], 200.0);
    near(hi[0], 300.0);
    near(hi[1], 424.0);
    // Identity, review metadata and unknown keys survive.
    assert_eq!(text_of(&d, b"NM"), "quote-1");
    assert_eq!(text_of(&d, b"T"), "Ada");
    assert_eq!(text_of(&d, b"CreationDate"), "D:20261001120000Z");
    assert_eq!(text_of(&d, b"M"), "D:20261002143000Z", "only the native modified date changes");
    assert_eq!(d.name(b"Vendor"), Some(&b"Acme"[..]));
    // The reply still points at the same annotation object.
    let (r, _) = annot(&doc, 0, h);
    let entries = annots(&doc, page_refs(&doc).unwrap()[0]);
    let reply = entries.iter().any(|o| o.as_ref().is_some_and(|e| doc.get(e).as_dict().is_some_and(|d| d.reference(b"IRT") == Some(r))));
    assert!(reply, "the /IRT reply survives");
}

fn text_of(d: &Dict, k: &[u8]) -> String {
    d.get(k).and_then(|o| o.as_string()).map(|s| s.to_text()).unwrap_or_default()
}

#[test]
fn resize_highlight_redraws_both_quads_in_the_appearance() {
    let mut doc = doc();
    let h = reviewed_highlight(&mut doc);
    let (r, d) = annot(&doc, 0, h);
    let ap_before = ap_n(&doc, &d);
    let _ = r;
    set_rect(&mut doc, 0, h, [50.0, 300.0, 250.0, 420.0], &meta("")).unwrap();
    let (_, d) = annot(&doc, 0, h);
    let ap = ap_content(&doc, &d);
    assert_eq!(ap.matches("h f").count(), 2, "one filled quad per line: {ap}");
    // The old appearance is replaced, not scaled.
    assert_ne!(ap_n(&doc, &d), ap_before, "a new appearance stream is drawn");
}

#[test]
fn resize_strikeout_and_redact_marks_scale_their_quads() {
    let mut doc = doc();
    let q = vec![[72.0, 512.0, 200.0, 512.0, 72.0, 500.0, 200.0, 500.0]];
    let s = add_annotation(&mut doc, &new(0, Shape::TextMarkup { kind: Markup::StrikeOut, quads: q.clone() }), &meta("s")).unwrap();
    let rd =
        add_annotation(&mut doc, &new(0, Shape::Redact { quads: q, overlay: String::new(), look: OverlayLook::default() }), &meta("rd")).unwrap();
    set_rect(&mut doc, 0, s, [40.0, 60.0, 120.0, 84.0], &meta("")).unwrap();
    set_rect(&mut doc, 0, rd, [200.0, 60.0, 280.0, 84.0], &meta("")).unwrap();
    let doc = reopen(&doc);
    let (_, sd) = annot(&doc, 0, s);
    let (_, rdd) = annot(&doc, 0, rd);
    assert_eq!(sd.name(b"Subtype"), Some(&b"StrikeOut"[..]));
    assert_eq!(rdd.name(b"Subtype"), Some(&b"Redact"[..]));
    // x: [72, 200] -> [40, 120] (scale 10/16); y: [500, 512] -> [60, 84].
    let want = [40.0 + (72.0 - 72.0) / 128.0 * 80.0, 60.0 + 12.0 / 12.0 * 24.0];
    let sq = nums(&sd, b"QuadPoints");
    near(sq[0], want[0]);
    near(sq[1], want[1]);
    near(sq[2], 40.0 + (200.0 - 72.0) / 128.0 * 80.0);
    near(sq[7], 60.0);
    assert_eq!(nums(&rdd, b"Rect"), [200.0, 60.0, 280.0, 84.0]);
    // A redaction resize is a review-mark edit only: the mark is still an outline, nothing applied.
    let content = ap_content(&doc, &rdd);
    assert!(content.contains("h S"), "{content}");
    assert!(!content.contains("f\n"), "the redaction is not filled: {content}");
    assert_eq!(text_of(&rdd, b"Contents"), "Review renewal limits\nConfirm service coverage");
}

#[test]
fn resize_note_grows_the_icon_and_keeps_appearance_and_flags() {
    let mut doc = doc();
    let n = add_annotation(&mut doc, &new(0, Shape::Note { at: [100.0, 100.0], icon: NoteIcon::Comment }), &meta("note-1")).unwrap();
    let (_, d) = annot(&doc, 0, n);
    let ap_before = ap_n(&doc, &d);
    let flags_before = d.get(b"F").cloned();
    set_rect(&mut doc, 0, n, [100.0, 100.0, 180.0, 180.0], &meta("edit-2")).unwrap();
    let doc = reopen(&doc);
    let (_, d) = annot(&doc, 0, n);
    assert_eq!(nums(&d, b"Rect"), [100.0, 100.0, 180.0, 180.0]);
    // The icon's appearance is kept: viewers scale it from its bounding box into the new /Rect.
    assert_eq!(ap_n(&doc, &d), ap_before, "the note's appearance is preserved, not redrawn");
    assert_eq!(d.get(b"F"), flags_before.as_ref(), "flags (print and any NoZoom/NoRotate) survive");
    assert_eq!(d.name(b"Subtype"), Some(&b"Text"[..]));
    assert_eq!(text_of(&d, b"NM"), "note-1");
    assert_eq!(text_of(&d, b"M"), "D:20261001120000Z");
}

#[test]
fn invalid_geometry_refuses_and_changes_nothing() {
    let mut doc = doc();
    let h = reviewed_highlight(&mut doc);
    let (r, _) = annot(&doc, 0, h);
    for rect in [[f64::NAN, 0.0, 10.0, 10.0], [0.0, 0.0, 0.5, 400.0], [0.0, 0.0, 10.0, f64::INFINITY], [10.0, 10.0, 10.4, 400.0]] {
        let before = snap(&doc, r);
        assert!(matches!(set_rect(&mut doc, 0, h, rect, &meta("")), Err(AnnotError::Invalid(_))), "{rect:?}");
        assert_eq!(snap(&doc, r), before, "a refused resize leaves the dictionary as it was");
    }
    // Hostile /QuadPoints (a broken count, or a non-finite entry) are refused too.
    for quads in [
        Object::Array(vec![Object::Int(1), Object::Int(2), Object::Int(3)]),
        Object::Array(vec![
            Object::Real(1.0),
            Object::Real(f64::NAN),
            Object::Int(0),
            Object::Int(0),
            Object::Int(0),
            Object::Int(0),
            Object::Int(0),
            Object::Int(0),
        ]),
    ] {
        doc.update_dict(r, |d| d.set(b"QuadPoints".to_vec(), quads.clone())).unwrap();
        let before = snap(&doc, r);
        assert!(matches!(set_rect(&mut doc, 0, h, [0.0, 0.0, 200.0, 100.0], &meta("")), Err(AnnotError::Invalid(_))));
        assert_eq!(snap(&doc, r), before, "malformed quads change nothing");
    }
}

#[test]
fn locked_comments_still_refuse_resize() {
    let mut doc = doc();
    let quads = vec![[72.0, 712.0, 200.0, 712.0, 72.0, 700.0, 200.0, 700.0]];
    let h = add_annotation(&mut doc, &new(0, Shape::TextMarkup { kind: Markup::Highlight, quads }), &meta("h")).unwrap();
    let n = add_annotation(&mut doc, &new(0, Shape::Note { at: [100.0, 100.0], icon: NoteIcon::Note }), &meta("n")).unwrap();
    set_locked(&mut doc, 0, h, true).unwrap();
    set_locked(&mut doc, 0, n, true).unwrap();
    for idx in [h, n] {
        let (r, _) = annot(&doc, 0, idx);
        let before = snap(&doc, r);
        assert!(matches!(set_rect(&mut doc, 0, idx, [0.0, 0.0, 200.0, 100.0], &meta("")), Err(AnnotError::Invalid(_))));
        assert_eq!(snap(&doc, r), before, "a locked comment is untouched");
    }
}

#[test]
fn malformed_source_frame_is_not_repaired_or_scaled_outside_the_target() {
    for frame in [
        Object::Null,
        num_array(&[72.0, 700.0, 72.0, 712.0]),
        num_array(&[0.0, 0.0, 1e-310, 1e-310]),
        num_array(&[0.0, 0.0, 10.0, 10.0]),
        Object::Array(vec![72.into(), 700.into(), Object::Null, 200.into(), 712.into()]),
    ] {
        let mut doc = doc();
        let h = reviewed_highlight(&mut doc);
        let (r, _) = annot(&doc, 0, h);
        doc.update_dict(r, |d| d.set(b"Rect".to_vec(), frame.clone())).unwrap();
        let before = snap(&doc, r);
        assert!(matches!(set_rect(&mut doc, 0, h, [100.0, 200.0, 300.0, 424.0], &meta("")), Err(AnnotError::Invalid(_))), "frame: {frame:?}");
        assert_eq!(snap(&doc, r), before, "a malformed source frame must not be silently repaired");
    }
}

#[test]
fn indirect_coordinates_and_rect_only_redaction_keep_their_native_forms() {
    let mut doc = doc();
    let h = reviewed_highlight(&mut doc);
    let (r, d) = annot(&doc, 0, h);
    let points: Vec<Object> = nums(&d, b"QuadPoints").into_iter().map(|v| Object::Ref(doc.add(Object::Real(v)))).collect();
    let points = doc.add(Object::Array(points));
    let frame = doc.add(num_array(&[72.0, 700.0, 200.0, 712.0]));
    doc.update_dict(r, |d| {
        d.set(b"QuadPoints".to_vec(), Object::Ref(points));
        d.set(b"Rect".to_vec(), Object::Ref(frame));
    })
    .unwrap();
    set_rect(&mut doc, 0, h, [100.0, 200.0, 300.0, 424.0], &meta("")).unwrap();
    let (_, d) = annot(&doc, 0, h);
    assert_eq!(nums(&d, b"QuadPoints").len(), 16);
    for (actual, expected) in nums(&d, b"QuadPoints").iter().zip([100.0, 424.0, 300.0, 386.6666666666667]) {
        near(*actual, expected);
    }
    let redaction = add_annotation(
        &mut doc,
        &new(
            0,
            Shape::Redact {
                quads: vec![[72.0, 512.0, 200.0, 512.0, 72.0, 500.0, 200.0, 500.0]],
                overlay: String::new(),
                look: OverlayLook::default(),
            },
        ),
        &meta("rectangle-only-review"),
    )
    .unwrap();
    let (r, _) = annot(&doc, 0, redaction);
    doc.update_dict(r, |d| {
        d.remove(b"QuadPoints");
    })
    .unwrap();
    set_rect(&mut doc, 0, redaction, [100.0, 200.0, 300.0, 424.0], &meta("")).unwrap();
    let doc = reopen(&doc);
    let (_, d) = annot(&doc, 0, redaction);
    assert!(!d.contains(b"QuadPoints"), "the optional field is not synthesized");
    assert_eq!(nums(&d, b"Rect"), [100.0, 200.0, 300.0, 424.0]);
    assert!(ap_content(&doc, &d).contains("h S"));
}

#[test]
fn excessive_coordinates_and_foreign_overlay_refuse_without_changing_the_mark() {
    let mut doc = doc();
    let h = reviewed_highlight(&mut doc);
    let (r, _) = annot(&doc, 0, h);
    let quad = [72.0, 712.0, 200.0, 712.0, 72.0, 700.0, 200.0, 700.0];
    let large = doc.add(Object::Array(quad.into_iter().cycle().take(1_000_008).map(Object::Real).collect()));
    doc.update_dict(r, |d| d.set(b"QuadPoints".to_vec(), Object::Ref(large))).unwrap();
    let before = snap(&doc, r);
    assert!(matches!(set_rect(&mut doc, 0, h, [100.0, 200.0, 300.0, 424.0], &meta("")), Err(AnnotError::Invalid(_))));
    assert_eq!(snap(&doc, r), before, "an oversized array must not change the mark");

    let redaction = add_annotation(
        &mut doc,
        &new(0, Shape::Redact { quads: vec![quad], overlay: String::new(), look: OverlayLook::default() }),
        &meta("foreign-overlay"),
    )
    .unwrap();
    let (r, _) = annot(&doc, 0, redaction);
    doc.update_dict(r, |d| d.set(b"OverlayText".to_vec(), PdfString::text("保密"))).unwrap();
    let before = snap(&doc, r);
    assert!(matches!(set_rect(&mut doc, 0, redaction, [100.0, 200.0, 300.0, 424.0], &meta("")), Err(AnnotError::Undrawable(_))));
    assert_eq!(snap(&doc, r), before, "the foreign appearance must not become question marks");
}

#[test]
fn other_types_still_refuse_resize() {
    let mut doc = doc();
    let l = add_annotation(
        &mut doc,
        &new(0, Shape::Line { from: [0.0, 0.0], to: [10.0, 10.0], start: LineEnding::None, end: LineEnding::None }),
        &meta("l"),
    )
    .unwrap();
    assert!(matches!(set_rect(&mut doc, 0, l, [0.0, 0.0, 5.0, 5.0], &meta("")), Err(AnnotError::Invalid(_))));
}
