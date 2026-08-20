BEGIN;

CREATE TABLE source_registry (
    source_id text NOT NULL
        CHECK (
            source_id <> ''
            AND source_id = btrim(source_id)
            AND position(':' IN source_id) = 0
        ),
    network text NOT NULL
        CHECK (
            network <> ''
            AND network = btrim(network)
            AND position(':' IN network) = 0
        ),
    version integer NOT NULL
        CHECK (version >= 1),
    role text NOT NULL
        CHECK (
            role IN (
                'committed-primary',
                'committed-independent',
                'provisional-realtime',
                'reconciliation-snapshot',
                'historical-backfill',
                'attribution-enrichment',
                'discovery-only'
            )
        ),
    trust text NOT NULL
        CHECK (
            trust IN (
                'locally-verified-committed',
                'independent-committed',
                'reconciled-snapshot',
                'recovery-only',
                'third-party-provisional',
                'mempool-provisional'
            )
        ),
    operator text NOT NULL
        CHECK (operator <> '' AND operator = btrim(operator)),
    operator_kind text NOT NULL
        CHECK (
            operator_kind IN (
                'local-node',
                'independent-node',
                'official',
                'provider',
                'community'
            )
        ),
    dataset_version text
        CHECK (
            dataset_version IS NULL
            OR (
                dataset_version <> ''
                AND dataset_version = btrim(dataset_version)
            )
        ),
    retention_class text NOT NULL
        CHECK (
            retention_class IN (
                'raw-indefinite',
                'raw-hot-local',
                'raw-warm-object',
                'compacted-canonical',
                'unknown-quarantine'
            )
        ),
    redistribution text NOT NULL
        CHECK (
            redistribution IN (
                'private-operator-evidence',
                'internal-only',
                'field-restricted',
                'redistributable'
            )
        ),
    evidence_class text NOT NULL
        CHECK (
            evidence_class IN (
                'committed-block',
                'auxiliary-order-status',
                'auxiliary-book-diff',
                'auxiliary-ledger',
                'snapshot',
                'historical-block',
                'public-market-data',
                'provisional-feed',
                'provisional-mempool'
            )
        ),
    license_name text
        CHECK (
            license_name IS NULL
            OR (
                license_name <> ''
                AND license_name = btrim(license_name)
            )
        ),
    agreement_status text
        CHECK (
            agreement_status IS NULL
            OR agreement_status IN ('active', 'disabled', 'expired')
        ),
    agreement_expires_at timestamptz,
    valid_from timestamptz NOT NULL,
    valid_to timestamptz,
    PRIMARY KEY (source_id, network, version),
    CHECK (
        (
            role = 'committed-primary'
            AND trust = 'locally-verified-committed'
        )
        OR (
            role = 'committed-independent'
            AND trust = 'independent-committed'
        )
        OR (
            role = 'provisional-realtime'
            AND trust IN ('third-party-provisional', 'mempool-provisional')
        )
        OR (
            role = 'reconciliation-snapshot'
            AND trust = 'reconciled-snapshot'
        )
        OR (
            role = 'historical-backfill'
            AND trust = 'recovery-only'
        )
        OR (
            role = 'attribution-enrichment'
            AND trust = 'third-party-provisional'
        )
        OR (
            role = 'discovery-only'
            AND trust = 'third-party-provisional'
        )
    ),
    CHECK (
        role NOT IN ('committed-primary', 'committed-independent')
        OR evidence_class = 'committed-block'
    ),
    CHECK (
        (
            operator_kind <> 'provider'
            AND role <> 'attribution-enrichment'
            AND license_name IS NULL
            AND agreement_status IS NULL
            AND agreement_expires_at IS NULL
        )
        OR (
            (
                operator_kind = 'provider'
                OR role = 'attribution-enrichment'
            )
            AND license_name IS NOT NULL
            AND agreement_status IS NOT NULL
        )
    ),
    CHECK (valid_to IS NULL OR valid_to > valid_from)
);

CREATE UNIQUE INDEX source_registry_current_uidx
    ON source_registry (source_id, network)
    WHERE valid_to IS NULL;

CREATE TABLE source_capability_binding (
    source_id text NOT NULL,
    network text NOT NULL,
    source_version integer NOT NULL,
    capability_id text NOT NULL
        CHECK (capability_id <> '' AND capability_id = btrim(capability_id)),
    PRIMARY KEY (source_id, network, source_version, capability_id),
    FOREIGN KEY (source_id, network, source_version)
        REFERENCES source_registry (source_id, network, version)
        ON DELETE RESTRICT
);

CREATE TABLE source_endpoint_version (
    source_id text NOT NULL,
    network text NOT NULL,
    source_version integer NOT NULL,
    transport text NOT NULL
        CHECK (
            transport <> ''
            AND transport = btrim(transport)
        ),
    endpoint_version text NOT NULL
        CHECK (
            endpoint_version <> ''
            AND endpoint_version = btrim(endpoint_version)
        ),
    PRIMARY KEY (source_id, network, source_version, transport, endpoint_version),
    FOREIGN KEY (source_id, network, source_version)
        REFERENCES source_registry (source_id, network, version)
        ON DELETE RESTRICT
);

CREATE TABLE source_license_policy (
    source_id text NOT NULL,
    network text NOT NULL,
    source_version integer NOT NULL,
    license_name text NOT NULL
        CHECK (license_name <> '' AND license_name = btrim(license_name)),
    redistribution text NOT NULL
        CHECK (
            redistribution IN (
                'private-operator-evidence',
                'internal-only',
                'field-restricted',
                'redistributable'
            )
        ),
    PRIMARY KEY (source_id, network, source_version),
    FOREIGN KEY (source_id, network, source_version)
        REFERENCES source_registry (source_id, network, version)
        ON DELETE RESTRICT
);

CREATE TABLE source_health_policy (
    source_id text NOT NULL,
    network text NOT NULL,
    source_version integer NOT NULL,
    probe_interval_millis integer NOT NULL
        CHECK (probe_interval_millis >= 1),
    consecutive_failure_threshold integer NOT NULL
        CHECK (consecutive_failure_threshold >= 1),
    suppress_on_inactive_agreement boolean NOT NULL DEFAULT true,
    PRIMARY KEY (source_id, network, source_version),
    FOREIGN KEY (source_id, network, source_version)
        REFERENCES source_registry (source_id, network, version)
        ON DELETE RESTRICT
);

CREATE TABLE source_probe_result (
    source_id text NOT NULL
        CHECK (source_id <> '' AND source_id = btrim(source_id)),
    network text NOT NULL
        CHECK (network <> '' AND network = btrim(network)),
    probed_at timestamptz NOT NULL,
    probe_sequence integer NOT NULL
        CHECK (probe_sequence >= 0),
    outcome text NOT NULL
        CHECK (outcome IN ('ok', 'failed', 'skipped')),
    reason_code text
        CHECK (
            reason_code IS NULL
            OR (
                reason_code <> ''
                AND reason_code = btrim(reason_code)
            )
        ),
    latency_millis integer
        CHECK (latency_millis IS NULL OR latency_millis >= 0),
    PRIMARY KEY (source_id, network, probed_at, probe_sequence)
);

COMMIT;
