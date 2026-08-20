use hl_protocol::{
    ObservationClass, PublicationLane, SourceAdmission, SourceTrust, SourceTrustError,
};
use serde::Serialize;

const COMMITTED_CLASSES: [ObservationClass; 4] = [
    ObservationClass::CommittedBlock,
    ObservationClass::AuxiliaryOrderStatus,
    ObservationClass::AuxiliaryBookDiff,
    ObservationClass::AuxiliaryLedger,
];

fn expected_pair(trust: SourceTrust, class: ObservationClass) -> Option<PublicationLane> {
    match trust {
        SourceTrust::LocallyVerifiedCommitted | SourceTrust::IndependentCommitted => match class {
            ObservationClass::CommittedBlock => Some(PublicationLane::CommittedCandidate),
            ObservationClass::AuxiliaryOrderStatus
            | ObservationClass::AuxiliaryBookDiff
            | ObservationClass::AuxiliaryLedger => Some(PublicationLane::Reconciliation),
            ObservationClass::Snapshot
            | ObservationClass::HistoricalBlock
            | ObservationClass::PublicMarketData
            | ObservationClass::ProvisionalFeed
            | ObservationClass::ProvisionalMempool => None,
        },
        SourceTrust::ReconciledSnapshot => match class {
            ObservationClass::Snapshot => Some(PublicationLane::Reconciliation),
            ObservationClass::CommittedBlock
            | ObservationClass::AuxiliaryOrderStatus
            | ObservationClass::AuxiliaryBookDiff
            | ObservationClass::AuxiliaryLedger
            | ObservationClass::HistoricalBlock
            | ObservationClass::PublicMarketData
            | ObservationClass::ProvisionalFeed
            | ObservationClass::ProvisionalMempool => None,
        },
        SourceTrust::RecoveryOnly => match class {
            ObservationClass::HistoricalBlock => Some(PublicationLane::Recovery),
            ObservationClass::CommittedBlock
            | ObservationClass::AuxiliaryOrderStatus
            | ObservationClass::AuxiliaryBookDiff
            | ObservationClass::AuxiliaryLedger
            | ObservationClass::Snapshot
            | ObservationClass::PublicMarketData
            | ObservationClass::ProvisionalFeed
            | ObservationClass::ProvisionalMempool => None,
        },
        SourceTrust::ThirdPartyProvisional => match class {
            ObservationClass::PublicMarketData | ObservationClass::ProvisionalFeed => {
                Some(PublicationLane::Provisional)
            }
            ObservationClass::CommittedBlock
            | ObservationClass::AuxiliaryOrderStatus
            | ObservationClass::AuxiliaryBookDiff
            | ObservationClass::AuxiliaryLedger
            | ObservationClass::Snapshot
            | ObservationClass::HistoricalBlock
            | ObservationClass::ProvisionalMempool => None,
        },
        SourceTrust::MempoolProvisional => match class {
            ObservationClass::ProvisionalMempool => Some(PublicationLane::Mempool),
            ObservationClass::CommittedBlock
            | ObservationClass::AuxiliaryOrderStatus
            | ObservationClass::AuxiliaryBookDiff
            | ObservationClass::AuxiliaryLedger
            | ObservationClass::Snapshot
            | ObservationClass::HistoricalBlock
            | ObservationClass::PublicMarketData
            | ObservationClass::ProvisionalFeed => None,
        },
    }
}

fn assert_admitted(trust: SourceTrust, class: ObservationClass, lane: PublicationLane) {
    let admission = SourceAdmission::new(trust, class).expect("allowed pair");
    assert_eq!(admission.trust(), trust);
    assert_eq!(admission.observation_class(), class);
    assert_eq!(admission.publication_lane(), lane);
}

fn assert_incompatible(trust: SourceTrust, class: ObservationClass) {
    assert_eq!(
        SourceAdmission::new(trust, class).expect_err("unlisted pair must fail"),
        SourceTrustError::IncompatibleObservationClass
    );
}

fn pin_incompatible_classes(trust: SourceTrust, allowed: &[ObservationClass]) {
    for class in ObservationClass::ALL {
        if allowed.contains(&class) {
            continue;
        }
        match class {
            ObservationClass::CommittedBlock
            | ObservationClass::AuxiliaryOrderStatus
            | ObservationClass::AuxiliaryBookDiff
            | ObservationClass::AuxiliaryLedger
            | ObservationClass::Snapshot
            | ObservationClass::HistoricalBlock
            | ObservationClass::PublicMarketData
            | ObservationClass::ProvisionalFeed
            | ObservationClass::ProvisionalMempool => assert_incompatible(trust, class),
        }
    }
}

#[test]
fn every_trust_and_observation_class_pair_has_one_fail_closed_admission_outcome() {
    for trust in SourceTrust::ALL {
        for class in ObservationClass::ALL {
            match expected_pair(trust, class) {
                Some(expected_lane) => assert_admitted(trust, class, expected_lane),
                None => assert_incompatible(trust, class),
            }
        }
    }
}

#[test]
fn every_source_trust_arm_pins_constructible_pairs() {
    for trust in SourceTrust::ALL {
        match trust {
            SourceTrust::LocallyVerifiedCommitted => {
                assert_admitted(
                    trust,
                    ObservationClass::CommittedBlock,
                    PublicationLane::CommittedCandidate,
                );
                assert_admitted(
                    trust,
                    ObservationClass::AuxiliaryOrderStatus,
                    PublicationLane::Reconciliation,
                );
                assert_admitted(
                    trust,
                    ObservationClass::AuxiliaryBookDiff,
                    PublicationLane::Reconciliation,
                );
                assert_admitted(
                    trust,
                    ObservationClass::AuxiliaryLedger,
                    PublicationLane::Reconciliation,
                );
                pin_incompatible_classes(
                    trust,
                    &[
                        ObservationClass::CommittedBlock,
                        ObservationClass::AuxiliaryOrderStatus,
                        ObservationClass::AuxiliaryBookDiff,
                        ObservationClass::AuxiliaryLedger,
                    ],
                );
            }
            SourceTrust::IndependentCommitted => {
                assert_admitted(
                    trust,
                    ObservationClass::CommittedBlock,
                    PublicationLane::CommittedCandidate,
                );
                assert_admitted(
                    trust,
                    ObservationClass::AuxiliaryOrderStatus,
                    PublicationLane::Reconciliation,
                );
                assert_admitted(
                    trust,
                    ObservationClass::AuxiliaryBookDiff,
                    PublicationLane::Reconciliation,
                );
                assert_admitted(
                    trust,
                    ObservationClass::AuxiliaryLedger,
                    PublicationLane::Reconciliation,
                );
                pin_incompatible_classes(
                    trust,
                    &[
                        ObservationClass::CommittedBlock,
                        ObservationClass::AuxiliaryOrderStatus,
                        ObservationClass::AuxiliaryBookDiff,
                        ObservationClass::AuxiliaryLedger,
                    ],
                );
            }
            SourceTrust::ReconciledSnapshot => {
                assert_admitted(
                    trust,
                    ObservationClass::Snapshot,
                    PublicationLane::Reconciliation,
                );
                pin_incompatible_classes(trust, &[ObservationClass::Snapshot]);
            }
            SourceTrust::RecoveryOnly => {
                assert_admitted(
                    trust,
                    ObservationClass::HistoricalBlock,
                    PublicationLane::Recovery,
                );
                pin_incompatible_classes(trust, &[ObservationClass::HistoricalBlock]);
            }
            SourceTrust::ThirdPartyProvisional => {
                assert_admitted(
                    trust,
                    ObservationClass::PublicMarketData,
                    PublicationLane::Provisional,
                );
                assert_admitted(
                    trust,
                    ObservationClass::ProvisionalFeed,
                    PublicationLane::Provisional,
                );
                pin_incompatible_classes(
                    trust,
                    &[
                        ObservationClass::PublicMarketData,
                        ObservationClass::ProvisionalFeed,
                    ],
                );
            }
            SourceTrust::MempoolProvisional => {
                assert_admitted(
                    trust,
                    ObservationClass::ProvisionalMempool,
                    PublicationLane::Mempool,
                );
                pin_incompatible_classes(trust, &[ObservationClass::ProvisionalMempool]);
            }
        }
    }
}

#[test]
fn only_two_explicit_committed_block_pairings_can_advance_the_watermark() {
    let eligible = SourceTrust::ALL
        .into_iter()
        .flat_map(|trust| {
            ObservationClass::ALL
                .into_iter()
                .filter_map(move |class| SourceAdmission::new(trust, class).ok())
        })
        .filter(SourceAdmission::can_advance_committed_watermark)
        .map(|admission| (admission.trust(), admission.observation_class()))
        .collect::<Vec<_>>();

    assert_eq!(
        eligible,
        vec![
            (
                SourceTrust::LocallyVerifiedCommitted,
                ObservationClass::CommittedBlock
            ),
            (
                SourceTrust::IndependentCommitted,
                ObservationClass::CommittedBlock
            ),
        ]
    );
}

#[test]
fn committed_sources_keep_auxiliary_evidence_out_of_the_watermark_lane() {
    for trust in [
        SourceTrust::LocallyVerifiedCommitted,
        SourceTrust::IndependentCommitted,
    ] {
        for class in COMMITTED_CLASSES {
            let admission = SourceAdmission::new(trust, class).expect("committed source class");
            assert_eq!(
                admission.can_advance_committed_watermark(),
                class == ObservationClass::CommittedBlock
            );
        }
    }
}

#[test]
fn trust_values_have_stable_kebab_case_configuration_names() {
    #[derive(Serialize)]
    struct TrustConfig {
        trust: SourceTrust,
    }

    let serialized = SourceTrust::ALL
        .into_iter()
        .map(|trust| toml::to_string(&TrustConfig { trust }).expect("serialize trust"))
        .collect::<Vec<_>>();

    assert_eq!(
        serialized,
        vec![
            "trust = \"locally-verified-committed\"\n",
            "trust = \"independent-committed\"\n",
            "trust = \"reconciled-snapshot\"\n",
            "trust = \"recovery-only\"\n",
            "trust = \"third-party-provisional\"\n",
            "trust = \"mempool-provisional\"\n",
        ]
    );
}
