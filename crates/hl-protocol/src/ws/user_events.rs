use serde_json::{Map, Value};

use crate::SourceError;

const USER_EVENT_VARIANTS: &[(&str, UserEventKind)] = &[
    ("fills", UserEventKind::Fills),
    ("funding", UserEventKind::Funding),
    ("liquidation", UserEventKind::Liquidation),
    ("nonUserCancel", UserEventKind::NonUserCancel),
];

const LEDGER_DELTA_TYPES: &[&str] = &[
    "deposit",
    "withdraw",
    "internalTransfer",
    "subAccountTransfer",
    "liquidation",
    "vaultCreate",
    "vaultDeposit",
    "vaultDistribution",
    "vaultWithdraw",
    "vaultLeaderCommission",
    "spotTransfer",
    "accountClassTransfer",
    "spotGenesis",
    "rewardsClaim",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserEventKind {
    Fills,
    Funding,
    Liquidation,
    NonUserCancel,
}

pub fn classify_user_event(data: &Value) -> Result<UserEventKind, SourceError> {
    let object = data.as_object().ok_or_else(|| {
        SourceError::MalformedPayload("user event payload must be an object".to_owned())
    })?;
    let mut found = None;
    for key in object.keys() {
        match kind_for_key(key) {
            Some(kind) if found.is_none() => found = Some(kind),
            Some(_) => {
                return Err(SourceError::MalformedPayload(
                    "user event payload mixes variants".to_owned(),
                ));
            }
            None => {
                return Err(SourceError::SchemaDrift(format!(
                    "unknown websocket user-event variant {key}"
                )));
            }
        }
    }
    found.ok_or_else(|| SourceError::MalformedPayload("user event payload is empty".to_owned()))
}

pub fn classify_ledger_updates(updates: &Value) -> Result<(), SourceError> {
    let array = updates.as_array().ok_or_else(|| {
        SourceError::MalformedPayload("non-funding ledger updates must be an array".to_owned())
    })?;
    for update in array {
        let object = update.as_object().ok_or_else(|| {
            SourceError::MalformedPayload("ledger update must be an object".to_owned())
        })?;
        let delta = object
            .get("delta")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                SourceError::MalformedPayload("ledger update has no delta object".to_owned())
            })?;
        let type_name = delta.get("type").and_then(Value::as_str).ok_or_else(|| {
            SourceError::MalformedPayload("ledger delta has no type string".to_owned())
        })?;
        if !LEDGER_DELTA_TYPES.contains(&type_name) {
            return Err(SourceError::SchemaDrift(format!(
                "unknown websocket ledger-delta variant {type_name}"
            )));
        }
    }
    Ok(())
}

fn kind_for_key(key: &str) -> Option<UserEventKind> {
    USER_EVENT_VARIANTS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, kind)| *kind)
}

#[must_use]
pub fn object_has_state_affecting_key(object: &Map<String, Value>) -> bool {
    const KEYS: &[&str] = &[
        "fills",
        "funding",
        "liquidation",
        "nonUserCancel",
        "orders",
        "assetPositions",
        "clearinghouseState",
        "nonFundingLedgerUpdates",
        "twapSliceFills",
        "spotState",
        "userState",
        "perpDexStates",
    ];
    object.keys().any(|key| KEYS.contains(&key.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ws_liquidation_and_non_user_cancel_are_known_user_events() {
        assert_eq!(
            classify_user_event(&json!({"liquidation":{"lid":1}})).expect("liquidation"),
            UserEventKind::Liquidation
        );
        assert_eq!(
            classify_user_event(&json!({"nonUserCancel":[{"coin":"BTC","oid":1}]}))
                .expect("cancel"),
            UserEventKind::NonUserCancel
        );
    }

    #[test]
    fn ws_unknown_user_event_variant_is_schema_drift() {
        let error = classify_user_event(&json!({"mysteryEvent":{}})).expect_err("unknown");
        assert!(matches!(error, SourceError::SchemaDrift(_)));
    }
}
