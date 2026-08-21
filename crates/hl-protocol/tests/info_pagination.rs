use std::fs;
use std::path::{Path, PathBuf};

use hl_protocol::info::{
    TimePageCoverage, TimePageCursor, TimePageOutcome, TimePageRecord, TimeRangeGap,
};
use serde_json::Value;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/hyperliquid/official-info")
}

#[test]
fn info_identical_timestamps_do_not_skip_when_cursor_uses_overlap() {
    let raw = fs::read(fixture_root().join("response-same-ms-page.json")).expect("page");
    let page: Vec<Value> = serde_json::from_slice(&raw).expect("json");
    let encoded: Vec<Vec<u8>> = page
        .iter()
        .map(|row| serde_json::to_vec(row).expect("row"))
        .collect();
    let records: Vec<TimePageRecord<'_>> = page
        .iter()
        .zip(encoded.iter())
        .map(|(row, payload)| {
            TimePageRecord::new(
                row["time"].as_i64().expect("time"),
                row["tid"].as_str(),
                payload,
            )
            .expect("record")
        })
        .collect();

    let cursor = TimePageCursor::new(0, 1).expect("cursor");
    let TimePageOutcome::Next {
        cursor: first,
        records: kept,
    } = cursor.apply_page(&records[..2], 2).expect("first page")
    else {
        panic!("expected next page");
    };
    assert_eq!(kept, vec![0, 1]);
    assert_eq!(first.last_time_millis(), Some(100));
    assert_eq!(first.last_stable_id(), Some("b"));
    assert_eq!(first.next_query_start_millis(), 99);
    assert!(first.next_query_start_millis() <= first.last_time_millis().expect("last"));

    let TimePageOutcome::Exhausted {
        cursor: second,
        records: rest,
    } = first.apply_page(&records, 10).expect("overlap refetch")
    else {
        panic!("expected exhausted after overlapping refetch");
    };
    assert_eq!(rest, vec![2]);
    assert_eq!(second.last_stable_id(), Some("c"));
}

#[test]
fn info_last_timestamp_plus_one_would_drop_same_millisecond_records() {
    let a = br#"{"tid":"a"}"#;
    let b = br#"{"tid":"b"}"#;
    let records = [
        TimePageRecord::new(100, Some("a"), a).expect("a"),
        TimePageRecord::new(100, Some("b"), b).expect("b"),
    ];
    let cursor = TimePageCursor::new(100, 1).expect("cursor");
    let TimePageOutcome::Next { cursor, .. } = cursor.apply_page(&records[..1], 1).expect("page")
    else {
        panic!("next");
    };
    assert_ne!(cursor.next_query_start_millis(), 101);

    let plus_one_would_skip = records
        .iter()
        .filter(|record| record.time_millis() >= 101)
        .count();
    assert_eq!(plus_one_would_skip, 0);
    let rest = match cursor.apply_page(&records, 10).expect("overlap refetch") {
        TimePageOutcome::Next { records, .. } | TimePageOutcome::Exhausted { records, .. } => {
            records
        }
        TimePageOutcome::NoProgress => panic!("remaining same-ms record"),
    };
    assert_eq!(rest, vec![1]);
}

#[test]
fn info_repeated_full_page_without_cursor_progress_is_detected() {
    let a = br#"{"tid":"a"}"#;
    let b = br#"{"tid":"b"}"#;
    let records = [
        TimePageRecord::new(50, Some("a"), a).expect("a"),
        TimePageRecord::new(50, Some("b"), b).expect("b"),
    ];
    let cursor = TimePageCursor::new(0, 1).expect("cursor");
    let TimePageOutcome::Next { cursor, .. } = cursor.apply_page(&records, 2).expect("first")
    else {
        panic!("next");
    };
    assert!(matches!(
        cursor.apply_page(&records, 2).expect("stuck"),
        TimePageOutcome::NoProgress
    ));
}

#[test]
fn info_content_identity_dedupes_when_stable_id_is_absent() {
    let payload = br#"{"px":"1.0"}"#;
    let records = [
        TimePageRecord::new(7, None, payload).expect("one"),
        TimePageRecord::new(7, None, payload).expect("dup"),
        TimePageRecord::new(8, None, br#"{"px":"2.0"}"#).expect("later"),
    ];
    let cursor = TimePageCursor::new(0, 1).expect("cursor");
    let TimePageOutcome::Exhausted { records: kept, .. } =
        cursor.apply_page(&records, 10).expect("page")
    else {
        panic!("exhausted");
    };
    assert_eq!(kept, vec![0, 2]);
}

#[test]
fn info_same_ms_numeric_ids_are_not_dropped_across_page_boundary() {
    let id99: &[u8] = br#"{"tid":"99"}"#;
    let id100: &[u8] = br#"{"tid":"100"}"#;
    let id101: &[u8] = br#"{"tid":"101"}"#;
    let records = [
        TimePageRecord::new(100, Some("99"), id99).expect("99"),
        TimePageRecord::new(100, Some("100"), id100).expect("100"),
        TimePageRecord::new(100, Some("101"), id101).expect("101"),
    ];
    let cursor = TimePageCursor::new(0, 5).expect("cursor");
    let TimePageOutcome::Next {
        cursor: first,
        records: kept,
    } = cursor.apply_page(&records[..1], 1).expect("first page")
    else {
        panic!("expected next after a full first page");
    };
    assert_eq!(kept, vec![0]);
    assert_eq!(first.last_time_millis(), Some(100));
    assert_eq!(first.last_stable_id(), Some("99"));
    assert!(first.next_query_start_millis() <= 100);
    assert_ne!(first.next_query_start_millis(), 101);

    let rest = match first.apply_page(&records, 10).expect("overlap refetch") {
        TimePageOutcome::Next { records, .. } | TimePageOutcome::Exhausted { records, .. } => {
            records
        }
        TimePageOutcome::NoProgress => panic!("unseen same-ms ids 100 and 101 must surface"),
    };
    assert_eq!(rest, vec![1, 2]);

    let cursor_after = match first.apply_page(&records, 10).expect("second") {
        TimePageOutcome::Next { cursor, .. } | TimePageOutcome::Exhausted { cursor, .. } => cursor,
        TimePageOutcome::NoProgress => panic!("second page"),
    };
    assert!(matches!(
        cursor_after
            .apply_page(&records, 10)
            .expect("already-seen refetch"),
        TimePageOutcome::NoProgress
    ));
}

#[test]
fn info_same_ms_content_identity_keeps_unseen_hash_below_watermark() {
    let first_payload: &[u8] = br#"{"px":"1.0","note":"a"}"#;
    let second_payload: &[u8] = br#"{"px":"1.0","note":"b"}"#;
    assert_ne!(first_payload, second_payload);
    let records = [
        TimePageRecord::new(100, None, first_payload).expect("a"),
        TimePageRecord::new(100, None, second_payload).expect("b"),
    ];
    let id = |payload: &[u8]| format!("blake3:{}", hex::encode(blake3::hash(payload).as_bytes()));
    let larger = if id(first_payload) > id(second_payload) {
        0
    } else {
        1
    };
    let smaller = 1 - larger;
    let page_one = [records[larger]];
    let cursor = TimePageCursor::new(0, 1).expect("cursor");
    let TimePageOutcome::Next {
        cursor: first,
        records: kept,
    } = cursor.apply_page(&page_one, 1).expect("first page")
    else {
        panic!("expected next");
    };
    assert_eq!(kept, vec![0]);

    let rest = match first.apply_page(&records, 10).expect("overlap refetch") {
        TimePageOutcome::Next { records, .. } | TimePageOutcome::Exhausted { records, .. } => {
            records
        }
        TimePageOutcome::NoProgress => {
            panic!("unseen content identity that sorts below the seen hash must surface")
        }
    };
    assert_eq!(rest, vec![smaller]);
}

#[test]
fn info_coverage_records_truncation_earliest_time_and_gaps() {
    let gap = TimeRangeGap::new(10, 20).expect("gap");
    let coverage = TimePageCoverage::new(true, Some(20), vec![gap]).expect("coverage");
    assert!(coverage.truncated());
    assert_eq!(coverage.earliest_reliable_millis(), Some(20));
    assert_eq!(coverage.known_gaps()[0].start_millis(), 10);
    TimePageCoverage::new(false, Some(-1), Vec::new()).expect_err("negative");
    TimeRangeGap::new(5, 4).expect_err("inverted");
}
