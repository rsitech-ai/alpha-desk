use domain_types::{KnownTime, SourceId};
use hl_protocol::{
    AgreementStatus, NetworkId, ObservationClass, OperatorKind, ProviderLicense,
    RedistributionPolicy, RetentionClass, SourceCatalogError, SourceCatalogRecord,
    SourceDescriptor, SourceRole, SourceTrust, network_scoped_source_identity,
    observation_qualifies_committed_source, role_requires_provider_license,
};

fn network(name: &str) -> NetworkId {
    NetworkId::new(name).expect("network")
}

fn source_id(name: &str) -> SourceId {
    SourceId::new(name).expect("source")
}

fn time(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).expect("time")
}

fn license(name: &str, status: AgreementStatus, expires_at: Option<KnownTime>) -> ProviderLicense {
    ProviderLicense::new(name, status, expires_at).expect("license")
}

fn descriptor(source: &str, net: &str, role: SourceRole, operator: &str) -> SourceDescriptor {
    SourceDescriptor::new(
        source_id(source),
        network(net),
        role,
        operator,
        Some("dataset-v1".to_owned()),
        RetentionClass::RawHotLocal,
        RedistributionPolicy::InternalOnly,
    )
    .expect("descriptor")
}

fn record(
    source: &str,
    net: &str,
    role: SourceRole,
    operator_kind: OperatorKind,
    evidence: ObservationClass,
    license: Option<ProviderLicense>,
) -> Result<SourceCatalogRecord, SourceCatalogError> {
    SourceCatalogRecord::new(
        descriptor(source, net, role, "operator"),
        1,
        operator_kind,
        evidence,
        license,
        time(1),
        None,
    )
}

#[test]
fn committed_primary_and_discovery_only_roles_cannot_be_conflated() {
    assert_ne!(SourceRole::CommittedPrimary, SourceRole::DiscoveryOnly);
    assert_ne!(
        SourceRole::CommittedPrimary.trust(),
        SourceRole::DiscoveryOnly.trust()
    );
    assert!(SourceRole::CommittedPrimary.is_committed());
    assert!(!SourceRole::DiscoveryOnly.is_committed());
    assert!(SourceRole::CommittedPrimary.compatible_with(SourceTrust::LocallyVerifiedCommitted));
    assert!(!SourceRole::DiscoveryOnly.compatible_with(SourceTrust::LocallyVerifiedCommitted));
    assert!(!SourceRole::CommittedPrimary.compatible_with(SourceTrust::ThirdPartyProvisional));
    assert!(SourceRole::DiscoveryOnly.compatible_with(SourceTrust::ThirdPartyProvisional));
}

#[test]
fn every_source_role_maps_onto_source_trust() {
    let mapped = SourceRole::ALL
        .into_iter()
        .map(|role| (role, role.trust(), role.compatible_with(role.trust())))
        .collect::<Vec<_>>();
    assert_eq!(
        mapped,
        vec![
            (
                SourceRole::CommittedPrimary,
                SourceTrust::LocallyVerifiedCommitted,
                true
            ),
            (
                SourceRole::CommittedIndependent,
                SourceTrust::IndependentCommitted,
                true
            ),
            (
                SourceRole::ProvisionalRealtime,
                SourceTrust::ThirdPartyProvisional,
                true
            ),
            (
                SourceRole::ReconciliationSnapshot,
                SourceTrust::ReconciledSnapshot,
                true
            ),
            (
                SourceRole::HistoricalBackfill,
                SourceTrust::RecoveryOnly,
                true
            ),
            (
                SourceRole::AttributionEnrichment,
                SourceTrust::ThirdPartyProvisional,
                true
            ),
            (
                SourceRole::DiscoveryOnly,
                SourceTrust::ThirdPartyProvisional,
                true
            ),
        ]
    );

    for trust in SourceTrust::ALL {
        let role = SourceRole::from_trust(trust);
        assert!(
            role.compatible_with(trust),
            "{role:?} must remain compatible with {trust:?}"
        );
    }
}

#[test]
fn provider_source_requires_licensing_and_redistribution_policy_fields() {
    let error = record(
        "nansen-labels",
        "mainnet",
        SourceRole::AttributionEnrichment,
        OperatorKind::Provider,
        ObservationClass::PublicMarketData,
        None,
    )
    .expect_err("provider enrichment without a license");
    assert_eq!(error, SourceCatalogError::MissingProviderLicense);
    assert_eq!(
        error.reason_code(),
        "source_catalog.missing_provider_license"
    );

    let error = record(
        "quicknode-ws",
        "mainnet",
        SourceRole::ProvisionalRealtime,
        OperatorKind::Provider,
        ObservationClass::ProvisionalFeed,
        None,
    )
    .expect_err("provider operator without a license");
    assert_eq!(error, SourceCatalogError::MissingProviderLicense);

    let licensed = record(
        "nansen-labels",
        "mainnet",
        SourceRole::AttributionEnrichment,
        OperatorKind::Provider,
        ObservationClass::PublicMarketData,
        Some(license("nansen-api-tos", AgreementStatus::Active, None)),
    )
    .expect("licensed provider");
    assert_eq!(
        licensed.descriptor().redistribution(),
        RedistributionPolicy::InternalOnly
    );
    assert_eq!(
        licensed.license().expect("license").license_name(),
        "nansen-api-tos"
    );
    assert!(role_requires_provider_license(
        SourceRole::AttributionEnrichment,
        OperatorKind::Official
    ));
    assert!(role_requires_provider_license(
        SourceRole::ProvisionalRealtime,
        OperatorKind::Provider
    ));
    assert!(!role_requires_provider_license(
        SourceRole::DiscoveryOnly,
        OperatorKind::Community
    ));
}

#[test]
fn source_ids_are_stable_and_network_scoped() {
    let mainnet = descriptor(
        "nansen-labels",
        "mainnet",
        SourceRole::AttributionEnrichment,
        "nansen",
    );
    let again = descriptor(
        "nansen-labels",
        "mainnet",
        SourceRole::AttributionEnrichment,
        "nansen",
    );
    let testnet = descriptor(
        "nansen-labels",
        "testnet",
        SourceRole::AttributionEnrichment,
        "nansen",
    );

    assert_eq!(mainnet.stable_id(), again.stable_id());
    assert_eq!(mainnet.stable_id(), "mainnet:nansen-labels");
    assert_eq!(testnet.stable_id(), "testnet:nansen-labels");
    assert_ne!(mainnet.stable_id(), testnet.stable_id());
    assert_eq!(
        network_scoped_source_identity(mainnet.network(), mainnet.source_id()),
        mainnet.stable_id()
    );

    let colon_source = SourceId::new("mainnet:nansen-labels").expect("raw id allows colon");
    assert_eq!(
        SourceDescriptor::new(
            colon_source,
            network("mainnet"),
            SourceRole::DiscoveryOnly,
            "hypurrscan",
            None,
            RetentionClass::RawHotLocal,
            RedistributionPolicy::InternalOnly,
        )
        .expect_err("colon would collapse the scoped identity"),
        SourceCatalogError::InvalidSourceId
    );
    assert_eq!(
        NetworkId::new("main:net").expect_err("colon in network"),
        SourceCatalogError::InvalidNetwork
    );
}

#[test]
fn committed_source_requires_qualifying_evidence_class() {
    assert!(observation_qualifies_committed_source(
        ObservationClass::CommittedBlock
    ));
    for class in ObservationClass::ALL {
        let qualifies = observation_qualifies_committed_source(class);
        assert_eq!(qualifies, class == ObservationClass::CommittedBlock);
        for role in [
            SourceRole::CommittedPrimary,
            SourceRole::CommittedIndependent,
        ] {
            let result = record(
                "primary-node",
                "mainnet",
                role,
                OperatorKind::LocalNode,
                class,
                None,
            );
            if qualifies {
                result.expect("committed block qualifies");
            } else {
                assert_eq!(
                    result.expect_err("non-block evidence cannot mark committed"),
                    SourceCatalogError::MissingCommittedEvidence
                );
            }
        }
    }

    record(
        "hypurrscan",
        "mainnet",
        SourceRole::DiscoveryOnly,
        OperatorKind::Community,
        ObservationClass::PublicMarketData,
        None,
    )
    .expect("discovery-only does not need committed evidence");
}

#[test]
fn source_record_temporal_updates_preserve_history() {
    let first = record(
        "primary-node",
        "mainnet",
        SourceRole::CommittedPrimary,
        OperatorKind::LocalNode,
        ObservationClass::CommittedBlock,
        None,
    )
    .expect("v1");
    let closed = first.with_closed_validity(time(10)).expect("close v1");
    assert_eq!(closed.version(), 1);
    assert_eq!(closed.valid_to().map(KnownTime::unix_micros), Some(10));
    assert!(!closed.allows_scheduled_work(time(10)));

    let next_descriptor = SourceDescriptor::new(
        source_id("primary-node"),
        network("mainnet"),
        SourceRole::CommittedPrimary,
        "alpha-desk",
        Some("dataset-v2".to_owned()),
        RetentionClass::RawIndefinite,
        RedistributionPolicy::PrivateOperatorEvidence,
    )
    .expect("v2 descriptor");
    let second = first
        .successor(
            next_descriptor,
            OperatorKind::LocalNode,
            ObservationClass::CommittedBlock,
            None,
            time(10),
        )
        .expect("v2");
    assert_eq!(second.version(), 2);
    assert!(second.is_current());
    assert_eq!(second.descriptor().dataset_version(), Some("dataset-v2"));
    assert_eq!(first.descriptor().dataset_version(), Some("dataset-v1"));

    assert_eq!(
        first
            .successor(
                descriptor(
                    "other-node",
                    "mainnet",
                    SourceRole::CommittedPrimary,
                    "operator"
                ),
                OperatorKind::LocalNode,
                ObservationClass::CommittedBlock,
                None,
                time(10),
            )
            .expect_err("identity must stay put"),
        SourceCatalogError::ConflictingIdentity
    );
    assert_eq!(
        first
            .with_closed_validity(time(1))
            .expect_err("end must follow start"),
        SourceCatalogError::InvalidValidityWindow
    );
}

#[test]
fn disabled_and_expired_provider_agreements_suppress_scheduled_work() {
    let active = record(
        "nansen-labels",
        "mainnet",
        SourceRole::AttributionEnrichment,
        OperatorKind::Provider,
        ObservationClass::PublicMarketData,
        Some(license(
            "nansen-api-tos",
            AgreementStatus::Active,
            Some(time(100)),
        )),
    )
    .expect("active");
    assert!(active.allows_scheduled_work(time(99)));
    assert!(!active.allows_scheduled_work(time(100)));
    assert!(!active.allows_scheduled_work(time(101)));

    let disabled = record(
        "nansen-labels",
        "mainnet",
        SourceRole::AttributionEnrichment,
        OperatorKind::Provider,
        ObservationClass::PublicMarketData,
        Some(license("nansen-api-tos", AgreementStatus::Disabled, None)),
    )
    .expect("disabled");
    assert!(!disabled.allows_scheduled_work(time(1)));

    let expired = record(
        "nansen-labels",
        "mainnet",
        SourceRole::AttributionEnrichment,
        OperatorKind::Provider,
        ObservationClass::PublicMarketData,
        Some(license("nansen-api-tos", AgreementStatus::Expired, None)),
    )
    .expect("expired");
    assert!(!expired.allows_scheduled_work(time(1)));

    let node = record(
        "primary-node",
        "mainnet",
        SourceRole::CommittedPrimary,
        OperatorKind::LocalNode,
        ObservationClass::CommittedBlock,
        None,
    )
    .expect("node");
    assert!(node.allows_scheduled_work(time(1)));
}

#[test]
fn source_role_wire_names_are_stable_kebab_case() {
    assert_eq!(
        SourceRole::ALL.map(SourceRole::as_str),
        [
            "committed-primary",
            "committed-independent",
            "provisional-realtime",
            "reconciliation-snapshot",
            "historical-backfill",
            "attribution-enrichment",
            "discovery-only",
        ]
    );
}
