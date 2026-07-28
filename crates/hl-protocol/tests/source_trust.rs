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
    match (trust, class) {
        (
            SourceTrust::LocallyVerifiedCommitted | SourceTrust::IndependentCommitted,
            ObservationClass::CommittedBlock,
        ) => Some(PublicationLane::CommittedCandidate),
        (
            SourceTrust::LocallyVerifiedCommitted | SourceTrust::IndependentCommitted,
            ObservationClass::AuxiliaryOrderStatus
            | ObservationClass::AuxiliaryBookDiff
            | ObservationClass::AuxiliaryLedger,
        )
        | (SourceTrust::ReconciledSnapshot, ObservationClass::Snapshot) => {
            Some(PublicationLane::Reconciliation)
        }
        (SourceTrust::RecoveryOnly, ObservationClass::HistoricalBlock) => {
            Some(PublicationLane::Recovery)
        }
        (
            SourceTrust::ThirdPartyProvisional,
            ObservationClass::PublicMarketData | ObservationClass::ProvisionalFeed,
        ) => Some(PublicationLane::Provisional),
        (SourceTrust::MempoolProvisional, ObservationClass::ProvisionalMempool) => {
            Some(PublicationLane::Mempool)
        }
        _ => None,
    }
}

#[test]
fn every_trust_and_observation_class_pair_has_one_fail_closed_admission_outcome() {
    for trust in SourceTrust::ALL {
        for class in ObservationClass::ALL {
            let actual = SourceAdmission::new(trust, class);
            match expected_pair(trust, class) {
                Some(expected_lane) => {
                    let admission = actual.expect("allowed pair");
                    assert_eq!(admission.trust(), trust);
                    assert_eq!(admission.observation_class(), class);
                    assert_eq!(admission.publication_lane(), expected_lane);
                }
                None => assert_eq!(
                    actual.expect_err("unlisted pair must fail"),
                    SourceTrustError::IncompatibleObservationClass
                ),
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
