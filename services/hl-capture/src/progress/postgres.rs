use async_trait::async_trait;
use domain_types::{BlockHeight, ChainId, KnownTime, ManifestId};
use storage_ports::{
    ArchivedBlockPlan, CaptureCursor, CaptureProgressStore, PlannedPublication, ProgressError,
    ProgressRecordDisposition, PublicationAcknowledgement,
};
use tokio::sync::Mutex;
use tokio_postgres::{Client, GenericClient, IsolationLevel, Row};

const STORAGE_FAILURE: &str = "PostgreSQL progress operation failed";
const INVALID_DURABLE_ROW: &str = "PostgreSQL progress row is invalid";

#[derive(Debug)]
pub struct PostgresProgressStore {
    client: Mutex<Client>,
}

impl PostgresProgressStore {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self {
            client: Mutex::new(client),
        }
    }
}

#[async_trait]
impl CaptureProgressStore for PostgresProgressStore {
    async fn initialize_chain(
        &self,
        chain_id: &ChainId,
        first_block_height: BlockHeight,
    ) -> Result<ProgressRecordDisposition, ProgressError> {
        let first_height = encode_u64(first_block_height.get());
        let client = self.client.lock().await;
        let inserted = client
            .execute(
                "
                INSERT INTO capture_chain_progress (
                    chain_id,
                    first_block_height,
                    initialized_at_micros
                )
                VALUES (
                    $1,
                    $2::text::numeric,
                    round(extract(epoch FROM clock_timestamp()) * 1000000)
                )
                ON CONFLICT (chain_id) DO NOTHING
                ",
                &[&chain_id.as_str(), &first_height],
            )
            .await
            .map_err(storage_error)?;
        if inserted == 1 {
            return Ok(ProgressRecordDisposition::New);
        }
        let row = client
            .query_one(
                "
                SELECT first_block_height::text
                FROM capture_chain_progress
                WHERE chain_id = $1
                ",
                &[&chain_id.as_str()],
            )
            .await
            .map_err(storage_error)?;
        if decode_u64(&row, 0)? == first_block_height.get() {
            Ok(ProgressRecordDisposition::IdenticalDuplicate)
        } else {
            Err(ProgressError::ConflictingInitialization)
        }
    }

    async fn record_archived(
        &self,
        plan: &ArchivedBlockPlan,
    ) -> Result<ProgressRecordDisposition, ProgressError> {
        let mut client = self.client.lock().await;
        let transaction = client
            .build_transaction()
            .isolation_level(IsolationLevel::Serializable)
            .start()
            .await
            .map_err(storage_error)?;
        let first_height = lock_chain(&transaction, plan.chain_id()).await?;
        if plan.block_height().get() < first_height {
            return Err(ProgressError::BelowFirstHeight);
        }
        if let Some(existing) =
            load_plan_from(&transaction, plan.chain_id(), plan.block_height()).await?
        {
            let disposition = if existing == *plan {
                ProgressRecordDisposition::IdenticalDuplicate
            } else {
                return Err(ProgressError::ConflictingBlock);
            };
            transaction.commit().await.map_err(storage_error)?;
            return Ok(disposition);
        }

        let height = encode_u64(plan.block_height().get());
        let canonical_hash = plan.canonical_block_hash().to_vec();
        let object_hash = plan.archive_object_sha256().to_vec();
        let manifest_hash = plan.archive_manifest_sha256().to_vec();
        let schema_fingerprint = plan.archive_schema_fingerprint().to_vec();
        let publication_count = encode_u64(
            u64::try_from(plan.publications().len())
                .map_err(|_| ProgressError::InvalidInput("too many publications"))?,
        );
        let archived_at = plan.archived_at().unix_micros().to_string();
        transaction
            .execute(
                "
                INSERT INTO capture_archived_blocks (
                    chain_id,
                    block_height,
                    canonical_block_hash,
                    archive_receipt_id,
                    archive_manifest_id,
                    archive_object_hash,
                    archive_manifest_hash,
                    archive_schema_fingerprint,
                    publication_count,
                    state,
                    archived_at_micros
                )
                VALUES (
                    $1,
                    $2::text::numeric,
                    $3,
                    $4,
                    $5,
                    $6,
                    $7,
                    $8,
                    $9::text::numeric,
                    'archived_pending',
                    $10::text::numeric
                )
                ",
                &[
                    &plan.chain_id().as_str(),
                    &height,
                    &canonical_hash,
                    &plan.archive_receipt_id(),
                    &plan.archive_manifest_id().as_str(),
                    &object_hash,
                    &manifest_hash,
                    &schema_fingerprint,
                    &publication_count,
                    &archived_at,
                ],
            )
            .await
            .map_err(storage_error)?;
        for publication in plan.publications() {
            let ordinal = encode_u64(u64::from(publication.ordinal()));
            let publication_hash = publication.publication_sha256().to_vec();
            transaction
                .execute(
                    "
                    INSERT INTO capture_block_publications (
                        chain_id,
                        block_height,
                        publication_ordinal,
                        message_id,
                        subject,
                        publication_hash
                    )
                    VALUES (
                        $1,
                        $2::text::numeric,
                        $3::text::numeric,
                        $4,
                        $5,
                        $6
                    )
                    ",
                    &[
                        &plan.chain_id().as_str(),
                        &height,
                        &ordinal,
                        &publication.message_id(),
                        &publication.subject(),
                        &publication_hash,
                    ],
                )
                .await
                .map_err(storage_error)?;
        }
        transaction.commit().await.map_err(storage_error)?;
        Ok(ProgressRecordDisposition::New)
    }

    async fn record_acknowledgement(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
        acknowledgement: &PublicationAcknowledgement,
    ) -> Result<ProgressRecordDisposition, ProgressError> {
        let mut client = self.client.lock().await;
        let transaction = client
            .build_transaction()
            .isolation_level(IsolationLevel::Serializable)
            .start()
            .await
            .map_err(storage_error)?;
        lock_chain(&transaction, chain_id).await?;
        let height = encode_u64(block_height.get());
        let ordinal = encode_u64(u64::from(acknowledgement.ordinal()));
        let row = transaction
            .query_opt(
                "
                SELECT
                    message_id,
                    subject,
                    publication_hash,
                    ack_stream,
                    ack_stream_sequence::text,
                    ack_duplicate,
                    acknowledged_at_micros::text
                FROM capture_block_publications
                WHERE chain_id = $1
                  AND block_height = $2::text::numeric
                  AND publication_ordinal = $3::text::numeric
                FOR UPDATE
                ",
                &[&chain_id.as_str(), &height, &ordinal],
            )
            .await
            .map_err(storage_error)?
            .ok_or(ProgressError::UnknownBlock)?;
        let planned_message_id = get_string(&row, 0)?;
        let planned_subject = get_string(&row, 1)?;
        let planned_hash = decode_hash(&row, 2)?;
        if planned_message_id != acknowledgement.message_id()
            || planned_subject != acknowledgement.subject()
            || planned_hash != acknowledgement.publication_sha256()
        {
            return Err(ProgressError::AcknowledgementMismatch);
        }

        let existing_stream = get_optional_string(&row, 3)?;
        if let Some(stream) = existing_stream {
            let existing = PublicationAcknowledgement::try_new(
                acknowledgement.ordinal(),
                planned_message_id,
                planned_subject,
                planned_hash,
                stream,
                decode_optional_u64(&row, 4)?.ok_or(ProgressError::Storage(INVALID_DURABLE_ROW))?,
                get_optional_bool(&row, 5)?.ok_or(ProgressError::Storage(INVALID_DURABLE_ROW))?,
                decode_optional_time(&row, 6)?
                    .ok_or(ProgressError::Storage(INVALID_DURABLE_ROW))?,
            )?;
            if existing == *acknowledgement {
                transaction.commit().await.map_err(storage_error)?;
                return Ok(ProgressRecordDisposition::IdenticalDuplicate);
            }
            return Err(ProgressError::ConflictingAcknowledgement);
        }
        if decode_optional_u64(&row, 4)?.is_some()
            || get_optional_bool(&row, 5)?.is_some()
            || decode_optional_time(&row, 6)?.is_some()
        {
            return Err(ProgressError::Storage(INVALID_DURABLE_ROW));
        }

        let publication_hash = acknowledgement.publication_sha256().to_vec();
        let sequence = encode_u64(acknowledgement.stream_sequence());
        let acknowledged_at = acknowledgement.acknowledged_at().unix_micros().to_string();
        transaction
            .execute(
                "
                UPDATE capture_block_publications
                SET
                    ack_stream = $4,
                    ack_stream_sequence = $5::text::numeric,
                    ack_duplicate = $6,
                    acknowledged_at_micros = $7::text::numeric
                WHERE chain_id = $1
                  AND block_height = $2::text::numeric
                  AND publication_ordinal = $3::text::numeric
                  AND publication_hash = $8
                  AND ack_stream IS NULL
                ",
                &[
                    &chain_id.as_str(),
                    &height,
                    &ordinal,
                    &acknowledgement.stream(),
                    &sequence,
                    &acknowledgement.duplicate(),
                    &acknowledged_at,
                    &publication_hash,
                ],
            )
            .await
            .map_err(storage_error)?;
        transaction
            .execute(
                "
                UPDATE capture_archived_blocks AS block
                SET state = CASE
                    WHEN NOT EXISTS (
                        SELECT 1
                        FROM capture_block_publications AS publication
                        WHERE publication.chain_id = block.chain_id
                          AND publication.block_height = block.block_height
                          AND publication.ack_stream_sequence IS NULL
                    )
                    THEN 'acknowledged'
                    ELSE 'publishing'
                END
                WHERE block.chain_id = $1
                  AND block.block_height = $2::text::numeric
                  AND block.state <> 'quarantined'
                ",
                &[&chain_id.as_str(), &height],
            )
            .await
            .map_err(storage_error)?;
        transaction.commit().await.map_err(storage_error)?;
        Ok(ProgressRecordDisposition::New)
    }

    async fn advance_cursor(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
    ) -> Result<CaptureCursor, ProgressError> {
        let mut client = self.client.lock().await;
        let transaction = client
            .build_transaction()
            .isolation_level(IsolationLevel::Serializable)
            .start()
            .await
            .map_err(storage_error)?;
        let first_height = lock_chain(&transaction, chain_id).await?;
        if let Some(existing) = load_cursor_from(&transaction, chain_id, true).await? {
            if existing.committed_block_height() == block_height {
                transaction.commit().await.map_err(storage_error)?;
                return Ok(existing);
            }
            let expected = existing
                .committed_block_height()
                .get()
                .checked_add(1)
                .map(BlockHeight::new)
                .ok_or(ProgressError::CursorOverflow)?;
            if block_height != expected {
                return Err(ProgressError::NonContiguousAdvance {
                    expected,
                    actual: block_height,
                });
            }
        } else if block_height.get() != first_height {
            return Err(ProgressError::NonContiguousAdvance {
                expected: BlockHeight::new(first_height),
                actual: block_height,
            });
        }

        let plan = load_plan_from(&transaction, chain_id, block_height)
            .await?
            .ok_or(ProgressError::UnknownBlock)?;
        let height = encode_u64(block_height.get());
        let state = transaction
            .query_one(
                "
                SELECT state
                FROM capture_archived_blocks
                WHERE chain_id = $1
                  AND block_height = $2::text::numeric
                FOR UPDATE
                ",
                &[&chain_id.as_str(), &height],
            )
            .await
            .map_err(storage_error)
            .and_then(|row| get_string(&row, 0))?;
        if state != "acknowledged" {
            return Err(ProgressError::PublicationIncomplete);
        }
        let acknowledgements =
            load_acknowledgements_from(&transaction, chain_id, block_height).await?;
        if acknowledgements.len() != plan.publications().len() {
            return Err(ProgressError::PublicationIncomplete);
        }
        let updated_at = acknowledgements
            .iter()
            .map(PublicationAcknowledgement::acknowledged_at)
            .max()
            .ok_or(ProgressError::PublicationIncomplete)?;
        let old_cursor = load_cursor_from(&transaction, chain_id, true).await?;
        let cursor_version = match old_cursor {
            Some(ref cursor) => cursor
                .cursor_version()
                .checked_add(1)
                .ok_or(ProgressError::CursorOverflow)?,
            None => 1,
        };
        let canonical_hash = plan.canonical_block_hash().to_vec();
        let manifest_hash = plan.archive_manifest_sha256().to_vec();
        let version = encode_u64(cursor_version);
        let updated_at_micros = updated_at.unix_micros().to_string();
        transaction
            .execute(
                "
                INSERT INTO capture_sequencer_cursors (
                    chain_id,
                    committed_block_height,
                    canonical_block_hash,
                    archive_manifest_hash,
                    archive_receipt_id,
                    cursor_version,
                    updated_at,
                    updated_at_micros
                )
                VALUES (
                    $1,
                    $2::text::numeric,
                    $3,
                    $4,
                    $5,
                    $6::text::numeric,
                    to_timestamp($7::text::double precision / 1000000),
                    $7::text::numeric
                )
                ON CONFLICT (chain_id) DO UPDATE
                SET
                    committed_block_height = EXCLUDED.committed_block_height,
                    canonical_block_hash = EXCLUDED.canonical_block_hash,
                    archive_manifest_hash = EXCLUDED.archive_manifest_hash,
                    archive_receipt_id = EXCLUDED.archive_receipt_id,
                    cursor_version = EXCLUDED.cursor_version,
                    updated_at = EXCLUDED.updated_at,
                    updated_at_micros = EXCLUDED.updated_at_micros
                ",
                &[
                    &chain_id.as_str(),
                    &height,
                    &canonical_hash,
                    &manifest_hash,
                    &plan.archive_receipt_id(),
                    &version,
                    &updated_at_micros,
                ],
            )
            .await
            .map_err(storage_error)?;
        let cursor = CaptureCursor::try_new(
            chain_id.clone(),
            block_height,
            plan.canonical_block_hash(),
            plan.archive_receipt_id(),
            plan.archive_manifest_sha256(),
            cursor_version,
            updated_at,
        )?;
        transaction.commit().await.map_err(storage_error)?;
        Ok(cursor)
    }

    async fn load_cursor(
        &self,
        chain_id: &ChainId,
    ) -> Result<Option<CaptureCursor>, ProgressError> {
        let client = self.client.lock().await;
        require_chain(&*client, chain_id).await?;
        load_cursor_from(&*client, chain_id, false).await
    }

    async fn next_expected_height(&self, chain_id: &ChainId) -> Result<BlockHeight, ProgressError> {
        let client = self.client.lock().await;
        let first_height = require_chain_height(&*client, chain_id).await?;
        match load_cursor_from(&*client, chain_id, false).await? {
            Some(cursor) => cursor
                .committed_block_height()
                .get()
                .checked_add(1)
                .map(BlockHeight::new)
                .ok_or(ProgressError::CursorOverflow),
            None => Ok(BlockHeight::new(first_height)),
        }
    }

    async fn load_archived_block(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
    ) -> Result<Option<ArchivedBlockPlan>, ProgressError> {
        let client = self.client.lock().await;
        require_chain(&*client, chain_id).await?;
        load_plan_from(&*client, chain_id, block_height).await
    }

    async fn load_acknowledgements(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
    ) -> Result<Vec<PublicationAcknowledgement>, ProgressError> {
        let client = self.client.lock().await;
        require_chain(&*client, chain_id).await?;
        if load_plan_from(&*client, chain_id, block_height)
            .await?
            .is_none()
        {
            return Err(ProgressError::UnknownBlock);
        }
        load_acknowledgements_from(&*client, chain_id, block_height).await
    }

    async fn pending_blocks(
        &self,
        chain_id: &ChainId,
        limit: usize,
    ) -> Result<Vec<ArchivedBlockPlan>, ProgressError> {
        if limit == 0 {
            return Err(ProgressError::InvalidLimit);
        }
        let limit = i64::try_from(limit).map_err(|_| ProgressError::InvalidLimit)?;
        let client = self.client.lock().await;
        require_chain(&*client, chain_id).await?;
        let rows = client
            .query(
                "
                SELECT block.block_height::text
                FROM capture_archived_blocks AS block
                LEFT JOIN capture_sequencer_cursors AS cursor
                  ON cursor.chain_id = block.chain_id
                WHERE block.chain_id = $1
                  AND (
                    cursor.committed_block_height IS NULL
                    OR block.block_height > cursor.committed_block_height
                  )
                ORDER BY block.block_height
                LIMIT $2
                ",
                &[&chain_id.as_str(), &limit],
            )
            .await
            .map_err(storage_error)?;
        let mut plans = Vec::with_capacity(rows.len());
        for row in rows {
            let height = BlockHeight::new(decode_u64(&row, 0)?);
            plans.push(
                load_plan_from(&*client, chain_id, height)
                    .await?
                    .ok_or(ProgressError::Storage(INVALID_DURABLE_ROW))?,
            );
        }
        Ok(plans)
    }
}

async fn lock_chain<C>(client: &C, chain_id: &ChainId) -> Result<u64, ProgressError>
where
    C: GenericClient + Sync,
{
    let row = client
        .query_opt(
            "
            SELECT first_block_height::text
            FROM capture_chain_progress
            WHERE chain_id = $1
            FOR UPDATE
            ",
            &[&chain_id.as_str()],
        )
        .await
        .map_err(storage_error)?
        .ok_or(ProgressError::ChainNotInitialized)?;
    decode_u64(&row, 0)
}

async fn require_chain<C>(client: &C, chain_id: &ChainId) -> Result<(), ProgressError>
where
    C: GenericClient + Sync,
{
    require_chain_height(client, chain_id).await.map(|_| ())
}

async fn require_chain_height<C>(client: &C, chain_id: &ChainId) -> Result<u64, ProgressError>
where
    C: GenericClient + Sync,
{
    let row = client
        .query_opt(
            "
            SELECT first_block_height::text
            FROM capture_chain_progress
            WHERE chain_id = $1
            ",
            &[&chain_id.as_str()],
        )
        .await
        .map_err(storage_error)?
        .ok_or(ProgressError::ChainNotInitialized)?;
    decode_u64(&row, 0)
}

async fn load_plan_from<C>(
    client: &C,
    chain_id: &ChainId,
    block_height: BlockHeight,
) -> Result<Option<ArchivedBlockPlan>, ProgressError>
where
    C: GenericClient + Sync,
{
    let height = encode_u64(block_height.get());
    let Some(row) = client
        .query_opt(
            "
            SELECT
                canonical_block_hash,
                archive_receipt_id,
                archive_manifest_id,
                archive_object_hash,
                archive_manifest_hash,
                archive_schema_fingerprint,
                publication_count::text,
                archived_at_micros::text
            FROM capture_archived_blocks
            WHERE chain_id = $1
              AND block_height = $2::text::numeric
            ",
            &[&chain_id.as_str(), &height],
        )
        .await
        .map_err(storage_error)?
    else {
        return Ok(None);
    };
    let publication_rows = client
        .query(
            "
            SELECT
                publication_ordinal::text,
                message_id,
                subject,
                publication_hash
            FROM capture_block_publications
            WHERE chain_id = $1
              AND block_height = $2::text::numeric
            ORDER BY publication_ordinal
            ",
            &[&chain_id.as_str(), &height],
        )
        .await
        .map_err(storage_error)?;
    let mut publications = Vec::with_capacity(publication_rows.len());
    for publication in publication_rows {
        publications.push(PlannedPublication::try_new(
            decode_u32(&publication, 0)?,
            get_string(&publication, 1)?,
            get_string(&publication, 2)?,
            decode_hash(&publication, 3)?,
        )?);
    }
    let expected_publication_count = decode_u64(&row, 6)?;
    let actual_publication_count = u64::try_from(publications.len())
        .map_err(|_| ProgressError::Storage(INVALID_DURABLE_ROW))?;
    if actual_publication_count != expected_publication_count {
        return Err(ProgressError::Storage(INVALID_DURABLE_ROW));
    }
    Ok(Some(ArchivedBlockPlan::try_new(
        chain_id.clone(),
        block_height,
        decode_hash(&row, 0)?,
        get_string(&row, 1)?,
        ManifestId::new(get_string(&row, 2)?)
            .map_err(|_| ProgressError::Storage(INVALID_DURABLE_ROW))?,
        decode_hash(&row, 3)?,
        decode_hash(&row, 4)?,
        decode_hash(&row, 5)?,
        publications,
        decode_time(&row, 7)?,
    )?))
}

async fn load_acknowledgements_from<C>(
    client: &C,
    chain_id: &ChainId,
    block_height: BlockHeight,
) -> Result<Vec<PublicationAcknowledgement>, ProgressError>
where
    C: GenericClient + Sync,
{
    let height = encode_u64(block_height.get());
    let rows = client
        .query(
            "
            SELECT
                publication_ordinal::text,
                message_id,
                subject,
                publication_hash,
                ack_stream,
                ack_stream_sequence::text,
                ack_duplicate,
                acknowledged_at_micros::text
            FROM capture_block_publications
            WHERE chain_id = $1
              AND block_height = $2::text::numeric
              AND ack_stream_sequence IS NOT NULL
            ORDER BY publication_ordinal
            ",
            &[&chain_id.as_str(), &height],
        )
        .await
        .map_err(storage_error)?;
    rows.into_iter()
        .map(|row| {
            PublicationAcknowledgement::try_new(
                decode_u32(&row, 0)?,
                get_string(&row, 1)?,
                get_string(&row, 2)?,
                decode_hash(&row, 3)?,
                get_string(&row, 4)?,
                decode_u64(&row, 5)?,
                get_bool(&row, 6)?,
                decode_time(&row, 7)?,
            )
        })
        .collect()
}

async fn load_cursor_from<C>(
    client: &C,
    chain_id: &ChainId,
    for_update: bool,
) -> Result<Option<CaptureCursor>, ProgressError>
where
    C: GenericClient + Sync,
{
    let query = if for_update {
        "
        SELECT
            committed_block_height::text,
            canonical_block_hash,
            archive_receipt_id,
            archive_manifest_hash,
            cursor_version::text,
            updated_at_micros::text
        FROM capture_sequencer_cursors
        WHERE chain_id = $1
        FOR UPDATE
        "
    } else {
        "
        SELECT
            committed_block_height::text,
            canonical_block_hash,
            archive_receipt_id,
            archive_manifest_hash,
            cursor_version::text,
            updated_at_micros::text
        FROM capture_sequencer_cursors
        WHERE chain_id = $1
        "
    };
    let Some(row) = client
        .query_opt(query, &[&chain_id.as_str()])
        .await
        .map_err(storage_error)?
    else {
        return Ok(None);
    };
    Ok(Some(CaptureCursor::try_new(
        chain_id.clone(),
        BlockHeight::new(decode_u64(&row, 0)?),
        decode_hash(&row, 1)?,
        get_string(&row, 2)?,
        decode_hash(&row, 3)?,
        decode_u64(&row, 4)?,
        decode_time(&row, 5)?,
    )?))
}

fn encode_u64(value: u64) -> String {
    value.to_string()
}

fn decode_u64(row: &Row, index: usize) -> Result<u64, ProgressError> {
    get_string(row, index)?
        .parse()
        .map_err(|_| ProgressError::Storage(INVALID_DURABLE_ROW))
}

fn decode_u32(row: &Row, index: usize) -> Result<u32, ProgressError> {
    get_string(row, index)?
        .parse()
        .map_err(|_| ProgressError::Storage(INVALID_DURABLE_ROW))
}

fn decode_optional_u64(row: &Row, index: usize) -> Result<Option<u64>, ProgressError> {
    get_optional_string(row, index)?
        .map(|value| {
            value
                .parse()
                .map_err(|_| ProgressError::Storage(INVALID_DURABLE_ROW))
        })
        .transpose()
}

fn decode_hash(row: &Row, index: usize) -> Result<[u8; 32], ProgressError> {
    let bytes = row.try_get::<_, Vec<u8>>(index).map_err(storage_error)?;
    bytes
        .try_into()
        .map_err(|_| ProgressError::Storage(INVALID_DURABLE_ROW))
}

fn decode_time(row: &Row, index: usize) -> Result<KnownTime, ProgressError> {
    let micros = get_string(row, index)?
        .parse()
        .map_err(|_| ProgressError::Storage(INVALID_DURABLE_ROW))?;
    KnownTime::from_unix_micros(micros).map_err(|_| ProgressError::Storage(INVALID_DURABLE_ROW))
}

fn decode_optional_time(row: &Row, index: usize) -> Result<Option<KnownTime>, ProgressError> {
    get_optional_string(row, index)?
        .map(|value| {
            let micros = value
                .parse()
                .map_err(|_| ProgressError::Storage(INVALID_DURABLE_ROW))?;
            KnownTime::from_unix_micros(micros)
                .map_err(|_| ProgressError::Storage(INVALID_DURABLE_ROW))
        })
        .transpose()
}

fn get_string(row: &Row, index: usize) -> Result<String, ProgressError> {
    row.try_get(index).map_err(storage_error)
}

fn get_optional_string(row: &Row, index: usize) -> Result<Option<String>, ProgressError> {
    row.try_get(index).map_err(storage_error)
}

fn get_bool(row: &Row, index: usize) -> Result<bool, ProgressError> {
    row.try_get(index).map_err(storage_error)
}

fn get_optional_bool(row: &Row, index: usize) -> Result<Option<bool>, ProgressError> {
    row.try_get(index).map_err(storage_error)
}

fn storage_error<T>(_error: T) -> ProgressError {
    ProgressError::Storage(STORAGE_FAILURE)
}
