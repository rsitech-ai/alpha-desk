use std::process;

use domain_types::{BlockHeight, ChainId, KnownTime, ManifestId};
use hl_capture::progress::PostgresProgressStore;
use storage_ports::{
    ArchivedBlockPlan, CaptureProgressStore, PlannedPublication, ProgressError,
    ProgressRecordDisposition, PublicationAcknowledgement,
};
use tokio_postgres::{Client, NoTls};

fn known(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).expect("known time")
}

fn plan(chain: ChainId, height: u64, salt: u8) -> ArchivedBlockPlan {
    ArchivedBlockPlan::try_new(
        chain,
        BlockHeight::new(height),
        [salt; 32],
        format!("receipt-{height}-{salt}"),
        ManifestId::new(format!("manifest-{height}-{salt}")).expect("manifest ID"),
        [salt.wrapping_add(1); 32],
        [salt.wrapping_add(2); 32],
        [salt.wrapping_add(3); 32],
        vec![
            PlannedPublication::try_new(
                0,
                format!("blk-{height}"),
                "hl.v1.block.committed",
                [salt.wrapping_add(4); 32],
            )
            .expect("block publication"),
            PlannedPublication::try_new(
                1,
                format!("evt-{height}"),
                "hl.v1.event.fill",
                [salt.wrapping_add(5); 32],
            )
            .expect("event publication"),
        ],
        known(1_721_779_300_000_000),
    )
    .expect("archived block plan")
}

fn acknowledgement(
    plan: &ArchivedBlockPlan,
    ordinal: u32,
    sequence: u64,
) -> PublicationAcknowledgement {
    let publication = &plan.publications()[usize::try_from(ordinal).expect("ordinal fits usize")];
    PublicationAcknowledgement::try_new(
        ordinal,
        publication.message_id(),
        publication.subject(),
        publication.publication_sha256(),
        "HL_CANONICAL",
        sequence,
        false,
        known(1_721_779_300_000_100 + i64::from(ordinal)),
    )
    .expect("publication acknowledgement")
}

async fn connect(url: &str) -> (Client, tokio::task::JoinHandle<()>) {
    let (client, connection) = tokio_postgres::connect(url, NoTls)
        .await
        .expect("connect to selected PostgreSQL test server");
    let driver = tokio::spawn(async move {
        connection
            .await
            .expect("PostgreSQL connection driver remains healthy");
    });
    (client, driver)
}

async fn cleanup(client: &mut Client, chain: &ChainId) {
    let transaction = client.transaction().await.expect("begin test cleanup");
    transaction
        .execute(
            "DELETE FROM capture_sequencer_cursors WHERE chain_id = $1",
            &[&chain.as_str()],
        )
        .await
        .expect("delete test cursor");
    transaction
        .execute(
            "DELETE FROM capture_block_publications WHERE chain_id = $1",
            &[&chain.as_str()],
        )
        .await
        .expect("delete test publications");
    transaction
        .execute(
            "DELETE FROM capture_archived_blocks WHERE chain_id = $1",
            &[&chain.as_str()],
        )
        .await
        .expect("delete test blocks");
    transaction
        .execute(
            "DELETE FROM capture_chain_progress WHERE chain_id = $1",
            &[&chain.as_str()],
        )
        .await
        .expect("delete test chain");
    transaction.commit().await.expect("commit test cleanup");
}

#[tokio::test]
async fn postgres_progress_survives_reconnect_and_preserves_transition_contract() {
    let Ok(url) = std::env::var("ALPHA_DESK_POSTGRES_TEST_URL") else {
        eprintln!(
            "SKIP postgres_progress: set ALPHA_DESK_POSTGRES_TEST_URL to a migrated disposable database"
        );
        return;
    };
    let chain =
        ChainId::new(format!("progress-integration-{}", process::id())).expect("test chain ID");
    let first = plan(chain.clone(), 42, 7);
    let second = plan(chain.clone(), 43, 8);

    let (mut client, driver) = connect(&url).await;
    cleanup(&mut client, &chain).await;
    let store = PostgresProgressStore::new(client);
    assert_eq!(
        store
            .initialize_chain(&chain, BlockHeight::new(42))
            .await
            .expect("initialize chain"),
        ProgressRecordDisposition::New
    );
    assert_eq!(
        store
            .initialize_chain(&chain, BlockHeight::new(42))
            .await
            .expect("idempotent chain initialization"),
        ProgressRecordDisposition::IdenticalDuplicate
    );
    store.record_archived(&first).await.expect("first plan");
    store.record_archived(&second).await.expect("second plan");
    assert_eq!(
        store
            .record_archived(&first)
            .await
            .expect("idempotent plan"),
        ProgressRecordDisposition::IdenticalDuplicate
    );
    store
        .record_acknowledgement(
            &chain,
            BlockHeight::new(42),
            &acknowledgement(&first, 0, 100),
        )
        .await
        .expect("first partial acknowledgement");
    assert_eq!(
        store
            .load_acknowledgements(&chain, BlockHeight::new(42))
            .await
            .expect("load partial acknowledgements")
            .len(),
        1
    );
    assert_eq!(
        store
            .advance_cursor(&chain, BlockHeight::new(42))
            .await
            .expect_err("partial publication may not advance"),
        ProgressError::PublicationIncomplete
    );

    drop(store);
    driver.await.expect("join first connection driver");

    let (client, driver) = connect(&url).await;
    let store = PostgresProgressStore::new(client);
    assert_eq!(
        store
            .pending_blocks(&chain, 8)
            .await
            .expect("recover pending plans"),
        vec![first.clone(), second.clone()]
    );
    store
        .record_acknowledgement(
            &chain,
            BlockHeight::new(42),
            &acknowledgement(&first, 1, 101),
        )
        .await
        .expect("complete first block");
    let cursor = store
        .advance_cursor(&chain, BlockHeight::new(42))
        .await
        .expect("advance first cursor");
    assert_eq!(cursor.cursor_version(), 1);
    assert_eq!(
        store
            .advance_cursor(&chain, BlockHeight::new(42))
            .await
            .expect("idempotent cursor advance"),
        cursor
    );
    assert_eq!(
        store
            .pending_blocks(&chain, 8)
            .await
            .expect("only second block remains"),
        vec![second]
    );

    drop(store);
    driver.await.expect("join second connection driver");
    let (mut client, driver) = connect(&url).await;
    cleanup(&mut client, &chain).await;
    drop(client);
    driver.await.expect("join cleanup connection driver");
}
