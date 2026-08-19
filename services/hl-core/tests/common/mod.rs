use hl_core::{DEAD_LETTER_SCHEMA_V1, JetStreamReplayConfig};

pub fn valid_dead_letter_jsonl_of_len(total_bytes: usize) -> Vec<u8> {
    const MAX_RECORD_FILE_BYTES: usize = 4_096 + 1;
    let mut leftover = Vec::with_capacity(total_bytes);
    let mut n = 0u32;
    while leftover.len() < total_bytes {
        let remaining = total_bytes - leftover.len();
        let file_len = remaining.min(MAX_RECORD_FILE_BYTES);
        leftover.extend(valid_dead_letter_line(&format!("cap-{n:08}"), file_len));
        n += 1;
    }
    assert_eq!(leftover.len(), total_bytes);
    leftover
}

fn valid_dead_letter_line(message_id: &str, file_len: usize) -> Vec<u8> {
    let mut encoded = serde_json::to_vec(&serde_json::json!({
        "schema_version": DEAD_LETTER_SCHEMA_V1,
        "reason_code": "core.jetstream_transport",
        "subject": "hl.v1.connect.transport",
        "message_id": message_id,
        "payload_sha256": hex::encode([0x11; 32]),
        "block_hash": hex::encode([0x22; 32]),
        "consumer": JetStreamReplayConfig::default_durable_name(),
        "retry_count": 0,
        "failed_at_unix_micros": 1,
    }))
    .expect("existing record json");
    assert!(
        file_len > encoded.len() && file_len - 1 <= 4_096,
        "file_len {file_len} cannot hold a valid ExistingRecord line of {} bytes",
        encoded.len()
    );
    encoded.resize(file_len - 1, b' ');
    encoded.push(b'\n');
    encoded
}
