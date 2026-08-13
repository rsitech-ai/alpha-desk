use canonical_events::EventKind;

#[test]
fn canonical_event_kinds_do_not_include_book_snapshots_or_diffs() {
    assert!(EventKind::try_from("BookSnapshot").is_err());
    assert!(EventKind::try_from("BookDiff").is_err());
    assert!(EventKind::try_from("L4BookUpdated").is_err());
    assert!(
        !EventKind::ALL
            .iter()
            .any(|kind| kind.as_wire_name().to_ascii_lowercase().contains("book"))
    );
}
