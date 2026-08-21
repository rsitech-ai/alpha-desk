use bytes::Bytes;
use serde_json::{Map, Value};

use super::v1::{NodeRecordKind, NodeStreamKind, require_object, require_string, require_u64};
use crate::SourceError;

/// Official exchange action `type` names that appear in committed `replica_cmds`.
///
/// This is a read-only catalog of node records. It is not an `/exchange` client.
pub const ACTION_TYPE_NAMES: &[&str] = &[
    "order",
    "cancel",
    "cancelByCloid",
    "scheduleCancel",
    "modify",
    "batchModify",
    "updateLeverage",
    "topUpIsolatedOnlyMargin",
    "updateIsolatedMargin",
    "sendAsset",
    "agentSendAsset",
    "sendToEvmWithData",
    "usdSend",
    "spotSend",
    "withdraw3",
    "usdClassTransfer",
    "cDeposit",
    "cWithdraw",
    "tokenDelegate",
    "vaultTransfer",
    "hip3LiquidatorTransfer",
    "approveAgent",
    "approveBuilderFee",
    "twapOrder",
    "twapCancel",
    "reserveRequestWeight",
    "noop",
    "userDexAbstraction",
    "agentEnableDexAbstraction",
    "userSetAbstraction",
    "agentSetAbstraction",
    "userOutcome",
    "validatorL1Stream",
    "authorizeAqav2Role",
    "claimRewards",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionCoordinate {
    block_round: u64,
    bundle_index: u32,
    action_index: u32,
}

impl ActionCoordinate {
    #[must_use]
    pub const fn block_round(self) -> u64 {
        self.block_round
    }

    #[must_use]
    pub const fn bundle_index(self) -> u32 {
        self.bundle_index
    }

    #[must_use]
    pub const fn action_index(self) -> u32 {
        self.action_index
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedActionV1 {
    kind: &'static str,
    coordinate: ActionCoordinate,
}

impl SignedActionV1 {
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        self.kind
    }

    #[must_use]
    pub const fn coordinate(&self) -> ActionCoordinate {
        self.coordinate
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionBlockV1 {
    round: u64,
    parent_round: u64,
    proposer: String,
    time: String,
    actions: Vec<SignedActionV1>,
}

impl TransactionBlockV1 {
    #[must_use]
    pub const fn round(&self) -> u64 {
        self.round
    }

    #[must_use]
    pub const fn parent_round(&self) -> u64 {
        self.parent_round
    }

    #[must_use]
    pub fn proposer(&self) -> &str {
        &self.proposer
    }

    #[must_use]
    pub fn time(&self) -> &str {
        &self.time
    }

    #[must_use]
    pub fn actions(&self) -> &[SignedActionV1] {
        &self.actions
    }
}

pub fn classify_transaction_block(
    event: &Map<String, Value>,
) -> Result<NodeRecordKind, SourceError> {
    parse_transaction_fields(event)?;
    Ok(NodeRecordKind::TransactionBlock)
}

pub fn parse_transaction_block(payload: Bytes) -> Result<TransactionBlockV1, SourceError> {
    let record = super::v1::parse_node_record(NodeStreamKind::TransactionBlocks, payload)?;
    if record.kind() != NodeRecordKind::TransactionBlock {
        return Err(SourceError::MalformedPayload(
            "payload is not a transaction block".to_owned(),
        ));
    }
    let root: Value = serde_json::from_slice(record.payload())
        .map_err(|_| SourceError::MalformedPayload("node record is not valid JSON".to_owned()))?;
    let object = root.as_object().ok_or_else(|| {
        SourceError::MalformedPayload("node record root must be an object".to_owned())
    })?;
    parse_transaction_fields(object)
}

fn parse_transaction_fields(event: &Map<String, Value>) -> Result<TransactionBlockV1, SourceError> {
    let abci_block = require_object(event, "abci_block")?;
    let time = require_string(abci_block, "time")?.to_owned();
    let round = require_u64(abci_block, "round")?;
    let parent_round = require_u64(abci_block, "parent_round")?;
    let proposer = require_string(abci_block, "proposer")?.to_owned();
    let bundles = signed_action_bundles(event, abci_block)?;
    let mut actions = Vec::new();
    for (bundle_index, bundle) in bundles.iter().enumerate() {
        let bundle_index = u32::try_from(bundle_index).map_err(|_| {
            SourceError::MalformedPayload(
                "transaction block has too many action bundles".to_owned(),
            )
        })?;
        for (action_index, action) in signed_actions(bundle)?.iter().enumerate() {
            let action_index = u32::try_from(action_index).map_err(|_| {
                SourceError::MalformedPayload(
                    "transaction bundle has too many signed actions".to_owned(),
                )
            })?;
            let kind = action_type_name(action)?;
            actions.push(SignedActionV1 {
                kind,
                coordinate: ActionCoordinate {
                    block_round: round,
                    bundle_index,
                    action_index,
                },
            });
        }
    }
    Ok(TransactionBlockV1 {
        round,
        parent_round,
        proposer,
        time,
        actions,
    })
}

fn signed_action_bundles<'a>(
    root: &'a Map<String, Value>,
    abci_block: &'a Map<String, Value>,
) -> Result<&'a [Value], SourceError> {
    // ponytail: both-present and both-absent parse as zero actions so
    // `parse_transaction_block().actions()` stays load-bearing-empty. Fail-closed
    // ambiguity lives in `canonical_events::node_mapping::select_action_bundles`.
    // Promoting this to SchemaDrift without updating
    // `committed_mapper_accepts_the_current_nested_empty_bundle_shape_only_when_unambiguous`
    // panics that test's parse unwrap. Do not consume `.actions()` as the mapping path.
    let root_bundles = optional_array(root, "signed_action_bundles")?;
    let nested_bundles = optional_array(abci_block, "signed_action_bundles")?;
    match (root_bundles, nested_bundles) {
        (Some(_), Some(_)) => Ok(&[]),
        (Some(bundles), None) | (None, Some(bundles)) => Ok(bundles),
        (None, None) => Ok(&[]),
    }
}

fn optional_array<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<Option<&'a [Value]>, SourceError> {
    match object.get(field) {
        None => Ok(None),
        Some(Value::Array(values)) => Ok(Some(values)),
        Some(_) => Err(SourceError::MalformedPayload(format!(
            "{field} must be an array"
        ))),
    }
}

fn signed_actions(bundle: &Value) -> Result<&[Value], SourceError> {
    match bundle {
        Value::Array(pair) if pair.len() == 2 => match &pair[1] {
            Value::Object(object) => match object.get("signed_actions") {
                Some(Value::Array(actions)) => Ok(actions),
                Some(_) => Err(SourceError::MalformedPayload(
                    "signed_actions must be an array".to_owned(),
                )),
                None => Ok(&[]),
            },
            _ => Err(SourceError::MalformedPayload(
                "action bundle pair must end with an object".to_owned(),
            )),
        },
        Value::Object(object) => match object.get("signed_actions") {
            Some(Value::Array(actions)) => Ok(actions),
            Some(_) => Err(SourceError::MalformedPayload(
                "signed_actions must be an array".to_owned(),
            )),
            None => Ok(&[]),
        },
        _ => Err(SourceError::MalformedPayload(
            "signed action bundle has an invalid shape".to_owned(),
        )),
    }
}

fn action_type_name(signed: &Value) -> Result<&'static str, SourceError> {
    let object = signed.as_object().ok_or_else(|| {
        SourceError::MalformedPayload("signed action must be an object".to_owned())
    })?;
    let action = require_object(object, "action")?;
    let type_name = match action.get("type").and_then(Value::as_str) {
        Some(name) => name,
        None if action.len() == 1 => action
            .keys()
            .next()
            .map(String::as_str)
            .ok_or_else(|| SourceError::MalformedPayload("signed action is empty".to_owned()))?,
        None => {
            return Err(SourceError::MalformedPayload(
                "signed action has no type".to_owned(),
            ));
        }
    };
    ACTION_TYPE_NAMES
        .iter()
        .copied()
        .find(|known| *known == type_name)
        .ok_or_else(|| SourceError::SchemaDrift(format!("unknown node action type {type_name}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceError;

    fn block_with_action(action: Value) -> Bytes {
        let body = serde_json::json!({
            "abci_block": {
                "time": "2026-07-28T12:00:00.000000000",
                "round": 992814678_u64,
                "parent_round": 992814677_u64,
                "proposer": "0x5ac99df645f3414876c816caa18b2d234024b487"
            },
            "signed_action_bundles": [[
                "0xbundle",
                { "signed_actions": [{ "action": action, "nonce": 1_u64 }] }
            ]]
        });
        Bytes::from(serde_json::to_vec(&body).expect("block json"))
    }

    #[test]
    fn every_documented_action_type_parses_with_stable_coordinates() {
        for type_name in ACTION_TYPE_NAMES {
            let parsed = parse_transaction_block(block_with_action(serde_json::json!({
                "type": type_name
            })))
            .unwrap_or_else(|_| panic!("{type_name} must parse"));
            assert_eq!(parsed.round(), 992814678);
            assert_eq!(parsed.actions().len(), 1);
            assert_eq!(parsed.actions()[0].kind(), *type_name);
            let coordinate = parsed.actions()[0].coordinate();
            assert_eq!(coordinate.block_round(), 992814678);
            assert_eq!(coordinate.bundle_index(), 0);
            assert_eq!(coordinate.action_index(), 0);
            let tagged = parse_transaction_block(block_with_action(serde_json::json!({
                *type_name: {}
            })))
            .unwrap_or_else(|_| panic!("{type_name} internally tagged must parse"));
            assert_eq!(tagged.actions()[0].kind(), *type_name);
        }
        assert_eq!(ACTION_TYPE_NAMES.len(), 35);
    }

    #[test]
    fn unknown_action_type_is_schema_drift() {
        let error = parse_transaction_block(block_with_action(serde_json::json!({
            "type": "placeOrderNow"
        })))
        .expect_err("unknown action");
        assert!(matches!(error, SourceError::SchemaDrift(_)));
        assert_eq!(error.reason_code(), "source.schema_drift");
    }

    #[test]
    fn empty_signed_action_bundles_keep_block_coordinates() {
        let payload = Bytes::from_static(
            br#"{"abci_block":{"time":"2026-07-28T12:00:00.000000000","round":992814678,"parent_round":992814677,"proposer":"0x5ac99df645f3414876c816caa18b2d234024b487"},"signed_action_bundles":[]}"#,
        );
        let parsed = parse_transaction_block(payload).expect("empty bundles");
        assert_eq!(parsed.round(), 992814678);
        assert!(parsed.actions().is_empty());
    }

    #[test]
    fn replay_keeps_action_coordinates() {
        let payload = block_with_action(serde_json::json!({"type": "noop"}));
        let first = parse_transaction_block(payload.clone()).expect("first");
        let second = parse_transaction_block(payload).expect("second");
        assert_eq!(first, second);
        assert_eq!(first.actions()[0].coordinate().action_index(), 0);
    }
}
