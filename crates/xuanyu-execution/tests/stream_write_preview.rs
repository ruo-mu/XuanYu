use xuanyu_execution::preview_stream_write_args;

#[test]
fn previews_partial_file_write_content() {
    let preview = preview_stream_write_args(
        "file_write",
        r#"{"path":"a.txt","content":"hello"#,
        Some(""),
    )
    .expect("preview");
    assert_eq!(preview.path.as_deref(), Some("a.txt"));
    assert_eq!(preview.content.as_deref(), Some("hello"));
    assert_eq!(preview.add, 1);
    assert_eq!(preview.del, 0);
}

#[test]
fn provisional_add_before_path_resolves() {
    let preview = preview_stream_write_args(
        "file_write",
        r#"{"content":"line1\nline2\nline3"#,
        None,
    )
    .expect("preview");
    assert!(preview.path.is_none());
    assert_eq!(preview.add, 3);
    assert_eq!(preview.del, 0);
}

#[test]
fn complete_file_edit_counts_lines() {
    let preview = preview_stream_write_args(
        "file_edit",
        r#"{"path":"a.txt","old_text":"a\nb","new_text":"a\nb\nc"}"#,
        None,
    )
    .expect("preview");
    assert_eq!(preview.add, 3);
    assert_eq!(preview.del, 2);
}
