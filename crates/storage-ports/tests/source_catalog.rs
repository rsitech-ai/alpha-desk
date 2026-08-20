use std::cell::RefCell;
use std::collections::BTreeMap;

use domain_types::{KnownTime, SourceId};
use hl_protocol::{
    AgreementStatus, NetworkId, ObservationClass, OperatorKind, ProviderLicense,
    RedistributionPolicy, RetentionClass, SourceCatalogRecord, SourceDescriptor, SourceRole,
};
use storage_ports::{SourceCatalogStore, SourceCatalogStoreError};

#[derive(Default)]
struct MemoryCatalog {
    records: RefCell<BTreeMap<(String, String), Vec<SourceCatalogRecord>>>,
}

impl MemoryCatalog {
    fn key(network: &NetworkId, source_id: &SourceId) -> (String, String) {
        (network.as_str().to_owned(), source_id.as_str().to_owned())
    }
}

impl SourceCatalogStore for MemoryCatalog {
    fn publish(
        &self,
        record: SourceCatalogRecord,
    ) -> Result<SourceCatalogRecord, SourceCatalogStoreError> {
        if !record.is_current() {
            return Err(SourceCatalogStoreError::Conflict);
        }
        let mut records = self.records.borrow_mut();
        let key = Self::key(
            record.descriptor().network(),
            record.descriptor().source_id(),
        );
        let history = records.entry(key).or_default();
        let published = match history.last() {
            Some(current) if current.is_current() => {
                let closed = current
                    .with_closed_validity(record.valid_from())
                    .map_err(SourceCatalogStoreError::InvalidRecord)?;
                let next = current
                    .successor(
                        record.descriptor().clone(),
                        record.operator_kind(),
                        record.evidence_class(),
                        record.license().cloned(),
                        record.valid_from(),
                    )
                    .map_err(SourceCatalogStoreError::InvalidRecord)?;
                let last = history.len() - 1;
                history[last] = closed;
                history.push(next.clone());
                next
            }
            Some(_) => return Err(SourceCatalogStoreError::Conflict),
            None => {
                if record.version() != 1 {
                    return Err(SourceCatalogStoreError::Conflict);
                }
                history.push(record.clone());
                record
            }
        };
        Ok(published)
    }

    fn current(
        &self,
        network: &NetworkId,
        source_id: &SourceId,
    ) -> Result<Option<SourceCatalogRecord>, SourceCatalogStoreError> {
        Ok(self
            .history(network, source_id)?
            .into_iter()
            .find(SourceCatalogRecord::is_current))
    }

    fn history(
        &self,
        network: &NetworkId,
        source_id: &SourceId,
    ) -> Result<Vec<SourceCatalogRecord>, SourceCatalogStoreError> {
        Ok(self
            .records
            .borrow()
            .get(&Self::key(network, source_id))
            .cloned()
            .unwrap_or_default())
    }

    fn scheduled_work(
        &self,
        at: KnownTime,
    ) -> Result<Vec<SourceCatalogRecord>, SourceCatalogStoreError> {
        let mut scheduled = self
            .records
            .borrow()
            .values()
            .flatten()
            .filter(|record| record.allows_scheduled_work(at))
            .cloned()
            .collect::<Vec<_>>();
        scheduled.sort_by(|left, right| {
            left.descriptor()
                .stable_id()
                .cmp(&right.descriptor().stable_id())
        });
        Ok(scheduled)
    }
}

fn network(name: &str) -> NetworkId {
    NetworkId::new(name).expect("network")
}

fn source_id(name: &str) -> SourceId {
    SourceId::new(name).expect("source")
}

fn time(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).expect("time")
}

fn license(status: AgreementStatus, expires_at: Option<KnownTime>) -> ProviderLicense {
    ProviderLicense::new("nansen-api-tos", status, expires_at).expect("license")
}

fn provider_record(
    source: &str,
    net: &str,
    valid_from: i64,
    license: Option<ProviderLicense>,
) -> SourceCatalogRecord {
    SourceCatalogRecord::new(
        SourceDescriptor::new(
            source_id(source),
            network(net),
            SourceRole::AttributionEnrichment,
            "nansen",
            Some("labels-v1".to_owned()),
            RetentionClass::RawHotLocal,
            RedistributionPolicy::InternalOnly,
        )
        .expect("descriptor"),
        1,
        OperatorKind::Provider,
        ObservationClass::PublicMarketData,
        license,
        time(valid_from),
        None,
    )
    .expect("record")
}

fn node_record(source: &str, net: &str, valid_from: i64) -> SourceCatalogRecord {
    SourceCatalogRecord::new(
        SourceDescriptor::new(
            source_id(source),
            network(net),
            SourceRole::CommittedPrimary,
            "alpha-desk",
            Some("hyperliquid-node-v1".to_owned()),
            RetentionClass::RawIndefinite,
            RedistributionPolicy::PrivateOperatorEvidence,
        )
        .expect("descriptor"),
        1,
        OperatorKind::LocalNode,
        ObservationClass::CommittedBlock,
        None,
        time(valid_from),
        None,
    )
    .expect("record")
}

#[test]
fn temporal_updates_preserve_prior_versions() {
    let catalog = MemoryCatalog::default();
    let first = catalog
        .publish(node_record("primary-node", "mainnet", 1))
        .expect("v1");
    assert_eq!(first.version(), 1);

    let published = catalog
        .publish(node_record("primary-node", "mainnet", 10))
        .expect("v2");
    assert_eq!(published.version(), 2);
    assert_eq!(
        published.descriptor().dataset_version(),
        Some("hyperliquid-node-v1")
    );

    let history = catalog
        .history(&network("mainnet"), &source_id("primary-node"))
        .expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].version(), 1);
    assert_eq!(history[0].valid_to().map(KnownTime::unix_micros), Some(10));
    assert!(!history[0].is_current());
    assert_eq!(history[1].version(), 2);
    assert!(history[1].is_current());
    assert_eq!(
        catalog
            .current(&network("mainnet"), &source_id("primary-node"))
            .expect("current")
            .expect("row")
            .version(),
        2
    );
}

#[test]
fn network_scoped_identities_do_not_overwrite_each_other() {
    let catalog = MemoryCatalog::default();
    catalog
        .publish(node_record("primary-node", "mainnet", 1))
        .expect("mainnet");
    catalog
        .publish(node_record("primary-node", "testnet", 1))
        .expect("testnet");

    assert_eq!(
        catalog
            .history(&network("mainnet"), &source_id("primary-node"))
            .expect("mainnet history")
            .len(),
        1
    );
    assert_eq!(
        catalog
            .history(&network("testnet"), &source_id("primary-node"))
            .expect("testnet history")
            .len(),
        1
    );
}

#[test]
fn disabled_and_expired_agreements_are_absent_from_scheduled_work() {
    let catalog = MemoryCatalog::default();
    catalog
        .publish(node_record("primary-node", "mainnet", 1))
        .expect("node");
    catalog
        .publish(provider_record(
            "nansen-active",
            "mainnet",
            1,
            Some(license(AgreementStatus::Active, Some(time(50)))),
        ))
        .expect("active");
    catalog
        .publish(provider_record(
            "nansen-disabled",
            "mainnet",
            1,
            Some(license(AgreementStatus::Disabled, None)),
        ))
        .expect("disabled");
    catalog
        .publish(provider_record(
            "nansen-expired-status",
            "mainnet",
            1,
            Some(license(AgreementStatus::Expired, None)),
        ))
        .expect("expired status");
    catalog
        .publish(provider_record(
            "nansen-expired-clock",
            "mainnet",
            1,
            Some(license(AgreementStatus::Active, Some(time(10)))),
        ))
        .expect("expired clock");

    let scheduled = catalog
        .scheduled_work(time(20))
        .expect("schedule")
        .into_iter()
        .map(|record| record.descriptor().stable_id())
        .collect::<Vec<_>>();
    assert_eq!(
        scheduled,
        vec![
            "mainnet:nansen-active".to_owned(),
            "mainnet:primary-node".to_owned(),
        ]
    );

    let after_active_expiry = catalog
        .scheduled_work(time(50))
        .expect("later")
        .into_iter()
        .map(|record| record.descriptor().stable_id())
        .collect::<Vec<_>>();
    assert_eq!(after_active_expiry, vec!["mainnet:primary-node".to_owned()]);
}

#[test]
fn first_publish_must_start_at_version_one() {
    let catalog = MemoryCatalog::default();
    let record = SourceCatalogRecord::new(
        node_record("primary-node", "mainnet", 1)
            .descriptor()
            .clone(),
        2,
        OperatorKind::LocalNode,
        ObservationClass::CommittedBlock,
        None,
        time(1),
        None,
    )
    .expect("constructed v2");
    assert_eq!(
        catalog
            .publish(record)
            .expect_err("empty history rejects v2"),
        SourceCatalogStoreError::Conflict
    );
}
