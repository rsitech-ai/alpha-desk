use super::message::WsObservation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotRelation {
    Initial,
    Duplicate,
    Replaced,
}

#[must_use]
pub fn relate_snapshots(
    previous: Option<blake3::Hash>,
    observation: &WsObservation,
) -> Option<SnapshotRelation> {
    let WsObservation::Snapshot(_) = observation else {
        return None;
    };
    Some(match previous {
        None => SnapshotRelation::Initial,
        Some(previous) if previous == observation.content_hash() => SnapshotRelation::Duplicate,
        Some(_) => SnapshotRelation::Replaced,
    })
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::ws::parse_ws_message;

    #[test]
    fn ws_duplicate_snapshot_matches_prior_hash() {
        let payload = Bytes::from_static(
            br#"{"channel":"userFills","data":{"isSnapshot":true,"user":"0x0000000000000000000000000000000000000001","fills":[]}}"#,
        );
        let first = parse_ws_message(payload.clone()).expect("first");
        let second = parse_ws_message(payload).expect("second");
        assert_eq!(
            relate_snapshots(None, &first),
            Some(SnapshotRelation::Initial)
        );
        assert_eq!(
            relate_snapshots(Some(first.content_hash()), &second),
            Some(SnapshotRelation::Duplicate)
        );
        assert_eq!(first, second);
    }
}
