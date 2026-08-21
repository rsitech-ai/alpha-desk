use crate::input::CoreInputSubject;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputDisposition {
    CommittedApply,
    ReconcileOnly,
    Quarantine,
}

impl InputDisposition {
    #[must_use]
    pub const fn from_subject(subject: CoreInputSubject) -> Self {
        match subject {
            CoreInputSubject::Committed(_) if subject.can_advance_committed_watermark() => {
                Self::CommittedApply
            }
            CoreInputSubject::Committed(_) => Self::ReconcileOnly,
            CoreInputSubject::HealthData
            | CoreInputSubject::HealthSource
            | CoreInputSubject::HealthModel => Self::ReconcileOnly,
            CoreInputSubject::BlockProvisional
            | CoreInputSubject::SnapshotAccount
            | CoreInputSubject::SnapshotMarket
            | CoreInputSubject::SnapshotEcosystem => Self::Quarantine,
        }
    }

    #[must_use]
    pub const fn may_enter_ledger(self) -> bool {
        matches!(self, Self::CommittedApply)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantineRecord {
    subject: CoreInputSubject,
    reason_code: &'static str,
}

impl QuarantineRecord {
    #[must_use]
    pub const fn new(subject: CoreInputSubject, reason_code: &'static str) -> Self {
        Self {
            subject,
            reason_code,
        }
    }

    #[must_use]
    pub const fn subject(&self) -> CoreInputSubject {
        self.subject
    }

    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        self.reason_code
    }
}

#[derive(Debug, Default)]
pub struct ReconciliationInbox {
    quarantined: Vec<QuarantineRecord>,
    reconciled: u64,
}

impl ReconciliationInbox {
    #[must_use]
    pub fn quarantined(&self) -> &[QuarantineRecord] {
        &self.quarantined
    }

    #[must_use]
    pub const fn reconciled(&self) -> u64 {
        self.reconciled
    }

    pub fn observe(&mut self, subject: CoreInputSubject) -> InputDisposition {
        let disposition = InputDisposition::from_subject(subject);
        match disposition {
            InputDisposition::CommittedApply => disposition,
            InputDisposition::ReconcileOnly => {
                self.reconciled = self.reconciled.saturating_add(1);
                disposition
            }
            InputDisposition::Quarantine => {
                let reason = if subject.is_provisional_lane() {
                    "core.quarantine.provisional_or_snapshot"
                } else {
                    "core.quarantine.non_committed"
                };
                self.quarantined
                    .push(QuarantineRecord::new(subject, reason));
                disposition
            }
        }
    }
}
