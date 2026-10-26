use super::*;

fn image(a: &mut Automation, doc: u64) -> Vec<u8> {
    a.call("page_render", &json!({ "doc": doc, "page": 1, "dpi": 72 }))
        .unwrap()
        .into_iter()
        .find_map(|content| if let Content::Png { data, .. } = content { Some(data) } else { None })
        .unwrap()
}

fn comment(a: &mut Automation, doc: u64) -> Value {
    let list = ok(a, "comment_list", json!({ "doc": doc }));
    assert_eq!(list["count"], 1);
    list["comments"][0].clone()
}

#[test]
fn review_resize_through_tools_retains_identity_ink_and_single_step_history() {
    let dir = workdir("native-review-resize");
    for kind in ["highlight", "strikeout", "note", "redact"] {
        let mut a = auto(&dir);
        let doc = ok(&mut a, "doc_open", json!({ "path": "a.pdf" }))["doc"].as_u64().unwrap();
        if kind == "redact" {
            ok(&mut a, "redact_mark", json!({ "doc": doc, "pages": [1], "find": "Page 1" }));
        } else if kind == "note" {
            ok(
                &mut a,
                "comment_add",
                json!({ "doc": doc, "page": 1, "type": kind, "at": [40,40],
                "contents": "Confirm annual maintenance coverage", "author": "Facilities Procurement" }),
            );
        } else {
            ok(
                &mut a,
                "comment_add",
                json!({ "doc": doc, "page": 1, "type": kind, "find": "Page 1",
                "contents": "Confirm annual maintenance coverage", "author": "Facilities Procurement" }),
            );
        }
        let before = comment(&mut a, doc);
        let original_image = image(&mut a, doc);
        let rect = json!([110.0, 100.0, 190.0, 160.0]);
        ok(&mut a, "comment_edit", json!({ "doc": doc, "id": before["id"], "rect": rect }));
        let resized = comment(&mut a, doc);
        assert_eq!(resized["id"], before["id"]);
        assert_eq!(resized["index"], before["index"]);
        assert_eq!(resized["author"], before["author"]);
        assert_eq!(resized["contents"], before["contents"]);
        assert_eq!(resized["rect"], rect);
        let resized_image = image(&mut a, doc);
        assert_ne!(resized_image, original_image, "{kind} must visibly move/resize");
        ok(&mut a, "edit_undo", json!({ "doc": doc }));
        assert_eq!(comment(&mut a, doc)["rect"], before["rect"], "one undo reverses exactly the resize");
        assert_eq!(image(&mut a, doc), original_image);
        ok(&mut a, "edit_redo", json!({ "doc": doc }));
        assert_eq!(comment(&mut a, doc)["rect"], rect);
        assert_eq!(image(&mut a, doc), resized_image);
        let path = format!("resized-{kind}.pdf");
        ok(&mut a, "doc_save", json!({ "doc": doc, "path": path }));
        let reopened = ok(&mut a, "doc_open", json!({ "path": path }))["doc"].as_u64().unwrap();
        assert_eq!(comment(&mut a, reopened)["id"], before["id"]);
        assert_eq!(comment(&mut a, reopened)["rect"], rect);
        assert_eq!(image(&mut a, reopened), resized_image);
    }
    std::fs::remove_dir_all(dir).unwrap();
}
