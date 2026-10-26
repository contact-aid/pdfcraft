//! Geometry-only review edits, before native appearance regeneration.
use super::*;

// Bound regenerated coordinate/appearance buffers before allocating them.
const MAX_COORDINATES: usize = 1_000_000;

fn invalid() -> AnnotError {
    AnnotError::Invalid("invalid text area or enclosing rectangle".into())
}

/// Scale every quad with one affine map from the enclosing source rectangle. Never repair a
/// missing/degenerate frame or let a quad extend outside the requested review area. Redaction
/// annotations may legally omit QuadPoints (§12.5.6.23); those keep their rectangle-only form.
pub(super) fn quads(doc: &mut Document, r: ObjRef, rect: [f64; 4], meta: &Meta) -> Result<(), AnnotError> {
    if !finite(&rect) || rect[2] - rect[0] < 1.0 || rect[3] - rect[1] < 1.0 {
        return Err(AnnotError::Invalid("invalid rectangle (too small)".into()));
    }
    let d = annot_dict(doc, r);
    let old = d.get(b"Rect").map(|o| doc.resolve(o)).ok_or_else(invalid)?;
    let old = old.as_array().filter(|a| a.len() == 4).ok_or_else(invalid)?;
    let mut frame = [0.0; 4];
    for (slot, value) in frame.iter_mut().zip(old) {
        *slot = doc.resolve(value).as_f64().ok_or_else(invalid)?;
    }
    if !finite(&frame) {
        return Err(invalid());
    }
    let frame = normalize(frame);
    if frame[2] <= frame[0] || frame[3] <= frame[1] {
        return Err(invalid());
    }
    let raw = d.get(b"QuadPoints").map(|o| doc.resolve(o));
    let scaled = if raw.as_ref().is_none_or(|o| matches!(o.as_ref(), Object::Null)) && d.name(b"Subtype") == Some(b"Redact") {
        None
    } else {
        refuse_foreign_text(doc, &d)?;
        let raw = raw.as_ref().and_then(|o| o.as_array()).ok_or_else(invalid)?;
        if raw.is_empty() || !raw.len().is_multiple_of(8) || raw.len() > MAX_COORDINATES {
            return Err(invalid());
        }
        let mut scaled = Vec::with_capacity(raw.len());
        for pair in raw.as_chunks::<2>().0 {
            // Each pair is a fixed-size array, so these constant indices cannot be out of bounds.
            let x = doc.resolve(&pair[0]).as_f64().ok_or_else(invalid)?;
            let y = doc.resolve(&pair[1]).as_f64().ok_or_else(invalid)?;
            if !finite(&[x, y]) || x < frame[0] || x > frame[2] || y < frame[1] || y > frame[3] {
                return Err(invalid());
            }
            // Positive finite extents and containment keep the interpolation factors in [0, 1],
            // including subnormal source extents. No overflowing extrapolation or axis fallback.
            let tx = (x - frame[0]) / (frame[2] - frame[0]);
            let ty = (y - frame[1]) / (frame[3] - frame[1]);
            let point = [(1.0 - tx) * rect[0] + tx * rect[2], (1.0 - ty) * rect[1] + ty * rect[3]];
            if !finite(&point) {
                return Err(invalid());
            }
            scaled.extend_from_slice(&point);
        }
        Some(scaled)
    };
    // An imported rectangle-only mark keeps its original appearance, scaled like a stamp.
    // The appearance builder needs quads; do not invent them or silently lose the existing look.
    if scaled.is_none()
        && !d.get(b"AP").is_some_and(|ap| {
            doc.resolve(ap)
                .as_dict()
                .is_some_and(|ap| ap.get(b"N").is_some_and(|n| matches!(doc.resolve(n).as_ref(), Object::Stream(_) | Object::Dict(_))))
        })
    {
        return Err(AnnotError::Unsupported("Redact".into()));
    }
    doc.update_dict(r, |d| {
        if let Some(scaled) = &scaled {
            d.set(b"QuadPoints".to_vec(), num_array(scaled));
        }
        d.set(b"Rect".to_vec(), num_array(&rect));
        touch(d, meta);
    })?;
    if scaled.is_none() { Ok(()) } else { redraw_or_restore(doc, r, d) }
}
