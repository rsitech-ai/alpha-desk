use std::collections::BTreeMap;
use std::sync::Mutex;

use domain_types::{ChainId, KnownTime, SourceId};
use hl_capture::{
    BackfillRequest, DatasetFormat, DatasetKind, EnumerationSpec, FsObjectStore, HistoricalArchive,
    HistoricalError, HistoricalFaultInjector, HistoricalFaultPoint, HistoricalProgressStore,
    ListedObject, MARKET_BUCKET, NODE_MAINNET_BUCKET, NoHistoricalFaults, ObjectBody, ObjectStore,
    PARSER_BUILD, RawPortHistoricalArchive, RequestPayer, enumerate_keys, hyperevm_limitations,
    import_objects, keys_in_range, l2_object_key, select_fills_dataset, select_trades_dataset,
};
use storage_ports::{HistoricalBackfillProgress, HistoricalGapStatus};

fn now() -> KnownTime {
    KnownTime::from_unix_micros(1_725_000_000_000_000).expect("time")
}

fn durable_archive(root: &std::path::Path) -> RawPortHistoricalArchive {
    RawPortHistoricalArchive::open(
        root.join("raw"),
        ChainId::new("mainnet").expect("chain"),
        SourceId::new("historical-s3").expect("source"),
        1_048_576,
    )
    .expect("raw historical archive")
}

fn fixture_store() -> FsObjectStore {
    FsObjectStore::new(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/hyperliquid/historical-s3"),
    )
}

struct OneShotHistoricalFault {
    point: Mutex<Option<HistoricalFaultPoint>>,
}

impl OneShotHistoricalFault {
    fn new(point: HistoricalFaultPoint) -> Self {
        Self {
            point: Mutex::new(Some(point)),
        }
    }
}

impl HistoricalFaultInjector for OneShotHistoricalFault {
    fn check(&self, point: HistoricalFaultPoint) -> Result<(), HistoricalError> {
        let mut selected = self.point.lock().expect("fault lock");
        if selected.as_ref() == Some(&point) {
            selected.take();
            Err(HistoricalError::InjectedFault(point))
        } else {
            Ok(())
        }
    }
}

struct TamperedEtagStore {
    inner: FsObjectStore,
}

impl ObjectStore for TamperedEtagStore {
    fn list(
        &self,
        bucket: &str,
        prefix: &str,
        start_key: &str,
        end_key: &str,
        payer: RequestPayer,
    ) -> Result<Vec<ListedObject>, HistoricalError> {
        self.inner.list(bucket, prefix, start_key, end_key, payer)
    }

    fn get(
        &self,
        bucket: &str,
        key: &str,
        payer: RequestPayer,
    ) -> Result<Option<ObjectBody>, HistoricalError> {
        let mut object = self.inner.get(bucket, key, payer)?;
        if let Some(body) = &mut object {
            body.etag = "tampered".to_owned();
        }
        Ok(object)
    }
}

struct OverlayStore {
    inner: FsObjectStore,
    overlays: BTreeMap<(String, String), Vec<u8>>,
}

impl ObjectStore for OverlayStore {
    fn list(
        &self,
        bucket: &str,
        prefix: &str,
        start_key: &str,
        end_key: &str,
        payer: RequestPayer,
    ) -> Result<Vec<ListedObject>, HistoricalError> {
        self.inner.list(bucket, prefix, start_key, end_key, payer)
    }

    fn get(
        &self,
        bucket: &str,
        key: &str,
        payer: RequestPayer,
    ) -> Result<Option<ObjectBody>, HistoricalError> {
        if let Some(bytes) = self.overlays.get(&(bucket.to_owned(), key.to_owned())) {
            let RequestPayer::Requester = payer;
            return Ok(Some(ObjectBody {
                key: key.to_owned(),
                etag: hex::encode(blake3::hash(bytes).as_bytes()),
                bytes: bytes.clone().into(),
            }));
        }
        self.inner.get(bucket, key, payer)
    }
}

#[test]
fn requester_pays_parse_fails_closed() {
    assert!(matches!(
        RequestPayer::parse("requester"),
        Ok(RequestPayer::Requester)
    ));
    assert!(matches!(
        RequestPayer::parse("owner"),
        Err(HistoricalError::RequesterPaysRequired)
    ));
    assert_eq!(
        RequestPayer::parse("").unwrap_err().reason_code(),
        "historical_s3.requester_pays_required"
    );
}

#[test]
fn key_range_enumeration_is_predictable() {
    let keys = enumerate_keys(&EnumerationSpec::L2Book {
        date: "20230916".to_owned(),
        hour: 9,
        coins: vec!["BTC".to_owned(), "SOL".to_owned()],
    })
    .expect("keys");
    assert_eq!(
        keys,
        vec![
            l2_object_key("20230916", 9, "BTC"),
            l2_object_key("20230916", 9, "SOL"),
        ]
    );
    let bounded = keys_in_range(
        &keys,
        &l2_object_key("20230916", 9, "SOL"),
        &l2_object_key("20230916", 9, "SOL"),
    )
    .expect("range");
    assert_eq!(bounded, vec![l2_object_key("20230916", 9, "SOL")]);
}

#[test]
fn old_and_new_dataset_formats_select_distinct_prefixes() {
    assert_eq!(
        select_fills_dataset(DatasetFormat::Current),
        DatasetKind::NodeFillsByBlock
    );
    assert_eq!(
        select_fills_dataset(DatasetFormat::Legacy),
        DatasetKind::NodeFillsLegacy
    );
    assert_eq!(
        select_trades_dataset(DatasetFormat::Legacy).expect("legacy trades"),
        DatasetKind::NodeTradesLegacy
    );
    assert!(matches!(
        select_trades_dataset(DatasetFormat::Current),
        Err(HistoricalError::FormatMismatch)
    ));
    DatasetKind::NodeFillsByBlock
        .validate_format(DatasetFormat::Legacy)
        .expect_err("current fills reject legacy");
    DatasetKind::L2Snapshots
        .validate_format(DatasetFormat::Current)
        .expect("l2 current");
}

#[test]
fn official_limitations_name_monthly_archives_and_missing_datasets() {
    let market = DatasetKind::L2Snapshots.official_limitations();
    assert!(market.contains("month"));
    assert!(market.contains("missing"));
    assert!(market.contains("Candles"));
    assert!(
        DatasetKind::NodeFillsLegacy
            .official_limitations()
            .contains("node_fills")
    );
    assert!(hyperevm_limitations().contains("T25"));
    assert!(matches!(
        DatasetKind::parse("hyperevm-blocks"),
        Err(HistoricalError::HyperEvmDeferred)
    ));
}

#[test]
fn missing_object_is_recorded_as_gap_and_present_object_is_archived() {
    let directory = tempfile::tempdir().expect("dir");
    let mut archive = durable_archive(directory.path());
    let progress = HistoricalProgressStore::memory();
    let keys = enumerate_keys(&EnumerationSpec::L2Book {
        date: "20230916".to_owned(),
        hour: 9,
        coins: vec!["BTC".to_owned(), "SOL".to_owned()],
    })
    .expect("keys");
    let report = import_objects(
        &fixture_store(),
        &mut archive,
        &progress,
        &NoHistoricalFaults,
        &BackfillRequest {
            dataset: DatasetKind::L2Snapshots,
            format: DatasetFormat::Current,
            bucket: MARKET_BUCKET.to_owned(),
            keys,
            request_payer: RequestPayer::Requester,
            imported_at: now(),
        },
    )
    .expect("import");
    assert_eq!(report.imported, 1);
    assert_eq!(report.gaps, 1);
    assert_eq!(report.parser_build, PARSER_BUILD);
    assert_eq!(
        report.coverage_start_key.as_deref(),
        Some(l2_object_key("20230916", 9, "BTC").as_str())
    );
    assert_eq!(
        report.coverage_end_key.as_deref(),
        Some(l2_object_key("20230916", 9, "SOL").as_str())
    );
    let sol = report
        .manifests
        .iter()
        .find(|manifest| manifest.key().ends_with("SOL.lz4"))
        .expect("sol");
    assert_eq!(sol.gap_status(), HistoricalGapStatus::Present);
    assert_eq!(sol.parser_build(), PARSER_BUILD);
    assert!(sol.requester_pays_cost().is_some());
    let btc = report
        .manifests
        .iter()
        .find(|manifest| manifest.key().ends_with("BTC.lz4"))
        .expect("btc");
    assert_eq!(btc.gap_status(), HistoricalGapStatus::MissingObject);
    let gaps = progress.load_gaps("l2-snapshots").expect("gaps");
    assert_eq!(gaps.len(), 1);
    assert!(gaps[0].key().ends_with("BTC.lz4"));
}

#[test]
fn etag_mismatch_fails_closed() {
    let directory = tempfile::tempdir().expect("dir");
    let mut archive = durable_archive(directory.path());
    let progress = HistoricalProgressStore::memory();
    let error = import_objects(
        &TamperedEtagStore {
            inner: fixture_store(),
        },
        &mut archive,
        &progress,
        &NoHistoricalFaults,
        &BackfillRequest {
            dataset: DatasetKind::L2Snapshots,
            format: DatasetFormat::Current,
            bucket: MARKET_BUCKET.to_owned(),
            keys: vec![l2_object_key("20230916", 9, "SOL")],
            request_payer: RequestPayer::Requester,
            imported_at: now(),
        },
    )
    .expect_err("etag");
    assert!(matches!(error, HistoricalError::HashMismatch));
}

#[test]
fn duplicate_object_import_is_idempotent_and_conflicting_payload_fails() {
    let directory = tempfile::tempdir().expect("dir");
    let mut archive = durable_archive(directory.path());
    let progress = HistoricalProgressStore::memory();
    let request = BackfillRequest {
        dataset: DatasetKind::AssetContexts,
        format: DatasetFormat::Current,
        bucket: MARKET_BUCKET.to_owned(),
        keys: vec!["asset_ctxs/20230916.csv.lz4".to_owned()],
        request_payer: RequestPayer::Requester,
        imported_at: now(),
    };
    let first = import_objects(
        &fixture_store(),
        &mut archive,
        &progress,
        &NoHistoricalFaults,
        &request,
    )
    .expect("first");
    assert_eq!(first.imported, 1);
    let second = import_objects(
        &fixture_store(),
        &mut archive,
        &progress,
        &NoHistoricalFaults,
        &request,
    )
    .expect("second");
    assert_eq!(second.imported, 0);
    assert_eq!(second.duplicates, 1);
    assert_eq!(second.manifests.len(), 1);

    let mut conflict_archive = durable_archive(&directory.path().join("conflict"));
    let conflict_progress = HistoricalProgressStore::memory();
    import_objects(
        &fixture_store(),
        &mut conflict_archive,
        &conflict_progress,
        &NoHistoricalFaults,
        &request,
    )
    .expect("seed");
    let error = import_objects(
        &OverlayStore {
            inner: fixture_store(),
            overlays: BTreeMap::from([(
                (
                    MARKET_BUCKET.to_owned(),
                    "asset_ctxs/20230916.csv.lz4".to_owned(),
                ),
                b"different-asset-ctx".to_vec(),
            )]),
        },
        &mut conflict_archive,
        &conflict_progress,
        &NoHistoricalFaults,
        &request,
    )
    .expect_err("conflict");
    assert!(matches!(error, HistoricalError::Conflict));
}

#[test]
fn earlier_range_after_later_cursor_is_imported_not_claimed_idle() {
    let directory = tempfile::tempdir().expect("dir");
    let mut archive = durable_archive(directory.path());
    let progress = HistoricalProgressStore::memory();
    let later = "node_fills_by_block/20230916/10";
    let earlier = "node_fills_by_block/20230801/1";
    let first = import_objects(
        &fixture_store(),
        &mut archive,
        &progress,
        &NoHistoricalFaults,
        &BackfillRequest {
            dataset: DatasetKind::NodeFillsByBlock,
            format: DatasetFormat::Current,
            bucket: NODE_MAINNET_BUCKET.to_owned(),
            keys: vec![later.to_owned()],
            request_payer: RequestPayer::Requester,
            imported_at: now(),
        },
    )
    .expect("later");
    assert_eq!(first.imported, 1);
    assert_eq!(
        progress
            .load_cursor("node_fills_by_block")
            .expect("cursor")
            .expect("present")
            .last_key(),
        later
    );

    let report = import_objects(
        &OverlayStore {
            inner: fixture_store(),
            overlays: BTreeMap::from([(
                (NODE_MAINNET_BUCKET.to_owned(), earlier.to_owned()),
                b"august-fill".to_vec(),
            )]),
        },
        &mut archive,
        &progress,
        &NoHistoricalFaults,
        &BackfillRequest {
            dataset: DatasetKind::NodeFillsByBlock,
            format: DatasetFormat::Current,
            bucket: NODE_MAINNET_BUCKET.to_owned(),
            keys: vec![earlier.to_owned()],
            request_payer: RequestPayer::Requester,
            imported_at: now(),
        },
    )
    .expect("earlier");
    assert_eq!(report.imported, 1);
    assert_eq!(report.duplicates, 0);
    assert_eq!(report.gaps, 0);
    assert_eq!(report.coverage_start_key.as_deref(), Some(earlier));
    assert_eq!(report.coverage_end_key.as_deref(), Some(earlier));
    assert_eq!(report.last_key.as_deref(), Some(earlier));
    assert!(
        progress
            .load_object("node_fills_by_block", earlier)
            .expect("object")
            .is_some()
    );
}

#[test]
fn crash_after_archive_resumes_without_a_second_body() {
    let directory = tempfile::tempdir().expect("dir");
    let persist = directory.path().join("progress.json");
    let faults = OneShotHistoricalFault::new(HistoricalFaultPoint::AfterArchive);
    {
        let mut archive = durable_archive(directory.path());
        let progress = HistoricalProgressStore::open(&persist).expect("progress");
        let error = import_objects(
            &fixture_store(),
            &mut archive,
            &progress,
            &faults,
            &BackfillRequest {
                dataset: DatasetKind::ReplicaCmds,
                format: DatasetFormat::Current,
                bucket: NODE_MAINNET_BUCKET.to_owned(),
                keys: vec!["replica_cmds/20230916/100".to_owned()],
                request_payer: RequestPayer::Requester,
                imported_at: now(),
            },
        )
        .expect_err("crash");
        assert!(matches!(
            error,
            HistoricalError::InjectedFault(HistoricalFaultPoint::AfterArchive)
        ));
        assert!(
            progress
                .load_cursor("replica_cmds")
                .expect("cursor")
                .is_none()
        );
    }

    let mut archive = durable_archive(directory.path());
    let progress = HistoricalProgressStore::open(&persist).expect("reopen");
    let report = import_objects(
        &fixture_store(),
        &mut archive,
        &progress,
        &NoHistoricalFaults,
        &BackfillRequest {
            dataset: DatasetKind::ReplicaCmds,
            format: DatasetFormat::Current,
            bucket: NODE_MAINNET_BUCKET.to_owned(),
            keys: vec!["replica_cmds/20230916/100".to_owned()],
            request_payer: RequestPayer::Requester,
            imported_at: now(),
        },
    )
    .expect("resume");
    assert_eq!(report.imported, 1);
    let replay = import_objects(
        &fixture_store(),
        &mut archive,
        &progress,
        &NoHistoricalFaults,
        &BackfillRequest {
            dataset: DatasetKind::ReplicaCmds,
            format: DatasetFormat::Current,
            bucket: NODE_MAINNET_BUCKET.to_owned(),
            keys: vec!["replica_cmds/20230916/100".to_owned()],
            request_payer: RequestPayer::Requester,
            imported_at: now(),
        },
    )
    .expect("replay");
    assert_eq!(replay.imported, 0);
    let body = archive
        .get(
            progress
                .load_object("replica_cmds", "replica_cmds/20230916/100")
                .expect("object")
                .expect("present")
                .archive_ref(),
        )
        .expect("read")
        .expect("bytes");
    assert_eq!(body.as_ref(), b"replica-cmd");
}

#[test]
fn node_format_prefixes_import_from_the_emulator() {
    let directory = tempfile::tempdir().expect("dir");
    let mut archive = durable_archive(directory.path());
    let progress = HistoricalProgressStore::memory();
    for (dataset, format, key) in [
        (
            DatasetKind::NodeFillsByBlock,
            DatasetFormat::Current,
            "node_fills_by_block/20230916/10",
        ),
        (
            DatasetKind::NodeFillsLegacy,
            DatasetFormat::Legacy,
            "node_fills/20230101/1",
        ),
        (
            DatasetKind::NodeTradesLegacy,
            DatasetFormat::Legacy,
            "node_trades/20230101/1",
        ),
        (
            DatasetKind::ExplorerBlocks,
            DatasetFormat::Current,
            "explorer_blocks/20230916/100",
        ),
    ] {
        let report = import_objects(
            &fixture_store(),
            &mut archive,
            &progress,
            &NoHistoricalFaults,
            &BackfillRequest {
                dataset,
                format,
                bucket: NODE_MAINNET_BUCKET.to_owned(),
                keys: vec![key.to_owned()],
                request_payer: RequestPayer::Requester,
                imported_at: now(),
            },
        )
        .unwrap_or_else(|_| panic!("{key}"));
        assert_eq!(report.imported, 1, "{key}");
        assert_eq!(report.parser_build, PARSER_BUILD, "{key}");
    }
}

#[test]
fn object_manifest_survives_progress_store_reopen() {
    let directory = tempfile::tempdir().expect("dir");
    let persist = directory.path().join("progress.json");
    let key = "replica_cmds/20230916/100";
    {
        let mut archive = durable_archive(directory.path());
        let progress = HistoricalProgressStore::open(&persist).expect("progress");
        let report = import_objects(
            &fixture_store(),
            &mut archive,
            &progress,
            &NoHistoricalFaults,
            &BackfillRequest {
                dataset: DatasetKind::ReplicaCmds,
                format: DatasetFormat::Current,
                bucket: NODE_MAINNET_BUCKET.to_owned(),
                keys: vec![key.to_owned()],
                request_payer: RequestPayer::Requester,
                imported_at: now(),
            },
        )
        .expect("import");
        assert_eq!(report.imported, 1);
        assert_eq!(report.manifests.len(), 1);
        assert_eq!(
            report.manifests[0].gap_status(),
            HistoricalGapStatus::Present
        );
    }

    let progress = HistoricalProgressStore::open(&persist).expect("reopen");
    let loaded = progress.load_manifests("replica_cmds").expect("manifests");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].key(), key);
    assert_eq!(loaded[0].gap_status(), HistoricalGapStatus::Present);
    assert!(loaded[0].first_event_time().is_some());
    assert!(loaded[0].first_block().is_some());
    assert!(loaded[0].byte_count() > 0);
    assert!(loaded[0].requester_pays_cost().is_some());
    assert!(loaded[0].content_hash().is_some());
}
