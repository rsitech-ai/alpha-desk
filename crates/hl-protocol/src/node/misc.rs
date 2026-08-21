use serde_json::{Map, Value};

use super::v1::{
    NodeRecordKind, require_singleton_object, require_string, require_u64, require_value_object,
};
use crate::SourceError;

pub const MISC_INNER_VARIANTS: &[&str] = &[
    "CDeposit",
    "Delegation",
    "CWithdrawal",
    "ValidatorRewards",
    "Funding",
    "LedgerUpdate",
];

pub const LEDGER_DELTA_VARIANTS: &[&str] = &[
    "Withdraw",
    "Deposit",
    "VaultCreate",
    "VaultDeposit",
    "VaultWithdraw",
    "VaultDistribution",
    "VaultLeaderCommission",
    "Liquidation",
    "InternalTransfer",
    "AccountClassTransfer",
    "SubAccountTransfer",
    "SpotTransfer",
    "SpotGenesis",
    "RewardsClaim",
    "AccountActivationGas",
    "PerpDexClassTransfer",
    "DeployGasAuction",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MiscEventCoordinate {
    block_number: Option<u64>,
    event_index: u32,
}

impl MiscEventCoordinate {
    #[must_use]
    pub const fn block_number(self) -> Option<u64> {
        self.block_number
    }

    #[must_use]
    pub const fn event_index(self) -> u32 {
        self.event_index
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MiscEventV1 {
    kind: NodeRecordKind,
    inner: &'static str,
    ledger_delta: Option<&'static str>,
    coordinate: MiscEventCoordinate,
}

impl MiscEventV1 {
    #[must_use]
    pub const fn kind(&self) -> NodeRecordKind {
        self.kind
    }

    #[must_use]
    pub const fn inner(&self) -> &'static str {
        self.inner
    }

    #[must_use]
    pub const fn ledger_delta(&self) -> Option<&'static str> {
        self.ledger_delta
    }

    #[must_use]
    pub const fn coordinate(&self) -> MiscEventCoordinate {
        self.coordinate
    }
}

pub fn classify_misc_event(event: &Map<String, Value>) -> Result<NodeRecordKind, SourceError> {
    Ok(parse_misc_fields(event, None, 0)?.kind)
}

pub fn parse_misc_event(
    event: &Map<String, Value>,
    block_number: Option<u64>,
    event_index: u32,
) -> Result<MiscEventV1, SourceError> {
    parse_misc_fields(event, block_number, event_index)
}

fn parse_misc_fields(
    event: &Map<String, Value>,
    block_number: Option<u64>,
    event_index: u32,
) -> Result<MiscEventV1, SourceError> {
    require_string(event, "time")?;
    require_string(event, "hash")?;
    let inner = require_singleton_object(event, "inner")?;
    let (variant, value) = inner
        .iter()
        .next()
        .ok_or_else(|| SourceError::MalformedPayload("misc event is empty".to_owned()))?;
    let inner_name = known_name(
        variant,
        MISC_INNER_VARIANTS,
        "unknown node misc-event variant",
    )?;
    let body = require_value_object(value, "misc event payload")?;
    let (kind, ledger_delta) = match inner_name {
        "CDeposit" => {
            require_string(body, "user")?;
            require_decimalish(body, "amount")?;
            (NodeRecordKind::MiscEvent, None)
        }
        "Delegation" => {
            require_string(body, "user")?;
            require_string(body, "validator")?;
            require_decimalish(body, "amount")?;
            require_bool(body, "is_undelegate")?;
            (NodeRecordKind::MiscEvent, None)
        }
        "CWithdrawal" => {
            require_string(body, "user")?;
            require_decimalish(body, "amount")?;
            require_bool(body, "is_finalized")?;
            (NodeRecordKind::MiscEvent, None)
        }
        "ValidatorRewards" => {
            require_pairs(body, "validator_to_reward")?;
            (NodeRecordKind::MiscEvent, None)
        }
        "Funding" => {
            require_string(body, "coin")?;
            require_decimalish(body, "usdc")?;
            require_decimalish(body, "szi")?;
            require_decimalish(body, "fundingRate")?;
            require_u64(body, "nSamples")?;
            (NodeRecordKind::MiscEvent, None)
        }
        "LedgerUpdate" => classify_ledger_update(body)?,
        _ => {
            return Err(SourceError::SchemaDrift(
                "unknown node misc-event variant".to_owned(),
            ));
        }
    };
    Ok(MiscEventV1 {
        kind,
        inner: inner_name,
        ledger_delta,
        coordinate: MiscEventCoordinate {
            block_number,
            event_index,
        },
    })
}

fn classify_ledger_update(
    ledger: &Map<String, Value>,
) -> Result<(NodeRecordKind, Option<&'static str>), SourceError> {
    let users = ledger
        .get("users")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SourceError::MalformedPayload("ledger update has no users array".to_owned())
        })?;
    if users.iter().any(|user| user.as_str().is_none()) {
        return Err(SourceError::MalformedPayload(
            "ledger update users must be strings".to_owned(),
        ));
    }
    let delta = require_singleton_object(ledger, "delta")?;
    let (variant, value) = delta
        .iter()
        .next()
        .ok_or_else(|| SourceError::MalformedPayload("ledger delta is empty".to_owned()))?;
    let delta_name = known_name(
        variant,
        LEDGER_DELTA_VARIANTS,
        "unknown node ledger-delta variant",
    )?;
    let body = require_value_object(value, "ledger delta payload")?;
    match delta_name {
        "Withdraw" => {
            require_decimalish(body, "usdc")?;
            require_u64(body, "nonce")?;
            require_decimalish(body, "fee")?;
        }
        "Deposit" => {
            require_decimalish(body, "usdc")?;
        }
        "VaultCreate" => {
            require_string(body, "vault")?;
            require_decimalish(body, "usdc")?;
            require_decimalish(body, "fee")?;
        }
        "VaultDeposit" | "VaultLeaderCommission" => {
            // ponytail: gitbook lists these variants without field shapes.
            // Accept any object until a qualified corpus documents fields.
        }
        "VaultWithdraw" => {
            require_string(body, "vault")?;
            require_string(body, "user")?;
            require_decimalish(body, "requestedUsd")?;
            require_decimalish(body, "commission")?;
            require_decimalish(body, "closingCost")?;
            require_decimalish(body, "basis")?;
        }
        "VaultDistribution" => {
            require_string(body, "vault")?;
            require_decimalish(body, "usdc")?;
        }
        "Liquidation" => {
            require_decimalish(body, "liquidatedNtlPos")?;
            require_decimalish(body, "accountValue")?;
            require_string(body, "leverageType")?;
            let positions = body
                .get("liquidatedPositions")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    SourceError::MalformedPayload(
                        "liquidation has no liquidatedPositions array".to_owned(),
                    )
                })?;
            for position in positions {
                let position = position.as_object().ok_or_else(|| {
                    SourceError::MalformedPayload(
                        "liquidated position must be an object".to_owned(),
                    )
                })?;
                require_string(position, "coin")?;
                require_decimalish(position, "szi")?;
            }
        }
        "InternalTransfer" => {
            require_decimalish(body, "usdc")?;
            require_string(body, "user")?;
            require_string(body, "destination")?;
            require_decimalish(body, "fee")?;
        }
        "AccountClassTransfer" => {
            require_decimalish(body, "usdc")?;
            require_bool(body, "toPerp")?;
        }
        "SubAccountTransfer" => {
            require_decimalish(body, "usdc")?;
            require_string(body, "user")?;
            require_string(body, "destination")?;
        }
        "SpotTransfer" => {
            require_string(body, "token")?;
            require_decimalish(body, "amount")?;
            require_decimalish(body, "usdcValue")?;
            require_string(body, "user")?;
            require_string(body, "destination")?;
            require_decimalish(body, "fee")?;
            require_decimalish(body, "nativeTokenFee")?;
        }
        "SpotGenesis" => {
            require_string(body, "token")?;
            require_decimalish(body, "amount")?;
        }
        "RewardsClaim" => {
            require_decimalish(body, "amount")?;
        }
        "AccountActivationGas" => {
            require_decimalish(body, "amount")?;
            require_string(body, "token")?;
        }
        "PerpDexClassTransfer" => {
            require_decimalish(body, "amount")?;
            require_string(body, "token")?;
            require_string(body, "dex")?;
            require_bool(body, "toPerp")?;
        }
        "DeployGasAuction" => {
            require_string(body, "token")?;
            require_decimalish(body, "amount")?;
        }
        _ => {
            return Err(SourceError::SchemaDrift(
                "unknown node ledger-delta variant".to_owned(),
            ));
        }
    }
    let kind = match delta_name {
        "Liquidation" => NodeRecordKind::Liquidation,
        "InternalTransfer"
        | "AccountClassTransfer"
        | "SubAccountTransfer"
        | "SpotTransfer"
        | "PerpDexClassTransfer" => NodeRecordKind::Transfer,
        _ => NodeRecordKind::MiscEvent,
    };
    Ok((kind, Some(delta_name)))
}

fn known_name(
    name: &str,
    known: &[&'static str],
    drift: &'static str,
) -> Result<&'static str, SourceError> {
    known
        .iter()
        .copied()
        .find(|candidate| *candidate == name)
        .ok_or_else(|| SourceError::SchemaDrift(drift.to_owned()))
}

fn require_bool(object: &Map<String, Value>, field: &str) -> Result<bool, SourceError> {
    object.get(field).and_then(Value::as_bool).ok_or_else(|| {
        SourceError::MalformedPayload(format!("node record has no boolean field {field}"))
    })
}

fn require_pairs(object: &Map<String, Value>, field: &str) -> Result<(), SourceError> {
    let pairs = object.get(field).and_then(Value::as_array).ok_or_else(|| {
        SourceError::MalformedPayload(format!("node record has no array field {field}"))
    })?;
    for pair in pairs {
        let pair = pair
            .as_array()
            .filter(|value| value.len() == 2)
            .ok_or_else(|| {
                SourceError::MalformedPayload(format!("{field} entries must be pairs"))
            })?;
        if pair[0].as_str().is_none() {
            return Err(SourceError::MalformedPayload(format!(
                "{field} pair key must be a string"
            )));
        }
        require_decimalish_value(&pair[1], field)?;
    }
    Ok(())
}

fn require_decimalish(object: &Map<String, Value>, field: &str) -> Result<(), SourceError> {
    let Some(value) = object.get(field) else {
        return Err(SourceError::MalformedPayload(format!(
            "node record has no field {field}"
        )));
    };
    require_decimalish_value(value, field)
}

fn require_decimalish_value(value: &Value, field: &str) -> Result<(), SourceError> {
    match value {
        Value::String(text) if !text.is_empty() => Ok(()),
        Value::Number(number) if number.as_u64().is_some() || number.as_i64().is_some() => Ok(()),
        Value::Number(_) => Err(SourceError::MalformedPayload(format!(
            "node record field {field} is a JSON float"
        ))),
        _ => Err(SourceError::MalformedPayload(format!(
            "node record field {field} must be a decimal string or integer"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceError;
    use serde_json::json;

    fn wrap(inner: Value) -> Map<String, Value> {
        json!({
            "time": "2026-07-28T12:00:01.000",
            "hash": "0x1111111111111111111111111111111111111111111111111111111111111111",
            "inner": inner
        })
        .as_object()
        .cloned()
        .expect("object")
    }

    fn ledger(delta: Value) -> Map<String, Value> {
        wrap(json!({
            "LedgerUpdate": {
                "users": ["0x2222222222222222222222222222222222222222"],
                "delta": delta
            }
        }))
    }

    #[test]
    fn every_documented_misc_inner_variant_parses() {
        let cases = [
            json!({"CDeposit": {"user": "0x22", "amount": "1.0"}}),
            json!({"Delegation": {
                "user": "0x22",
                "validator": "0x33",
                "amount": "1.0",
                "is_undelegate": false
            }}),
            json!({"CWithdrawal": {"user": "0x22", "amount": "1.0", "is_finalized": true}}),
            json!({"ValidatorRewards": {"validator_to_reward": [["0x33", "1"]]}}),
            json!({"Funding": {
                "coin": "BTC",
                "usdc": "1.0",
                "szi": "0.1",
                "fundingRate": "0.0001",
                "nSamples": 8
            }}),
        ];
        assert_eq!(MISC_INNER_VARIANTS.len(), 6);
        for inner in cases {
            let parsed = parse_misc_event(&wrap(inner.clone()), Some(42), 0)
                .unwrap_or_else(|_| panic!("{inner} must parse"));
            assert_eq!(parsed.coordinate().block_number(), Some(42));
            assert_eq!(parsed.coordinate().event_index(), 0);
        }
    }

    #[test]
    fn every_documented_ledger_delta_parses() {
        let cases = [
            json!({"Withdraw": {"usdc": "1.0", "nonce": 7, "fee": "0.1"}}),
            json!({"Deposit": {"usdc": "1.0"}}),
            json!({"VaultCreate": {"vault": "0x44", "usdc": "1.0", "fee": "0.1"}}),
            json!({"VaultDeposit": {"usdc": "1.0"}}),
            json!({"VaultWithdraw": {
                "vault": "0x44",
                "user": "0x22",
                "requestedUsd": "1.0",
                "commission": "0.0",
                "closingCost": "0.0",
                "basis": "1.0"
            }}),
            json!({"VaultDistribution": {"vault": "0x44", "usdc": "1.0"}}),
            json!({"VaultLeaderCommission": {"usdc": "0.1"}}),
            json!({"Liquidation": {
                "liquidatedNtlPos": "12500.25",
                "accountValue": "-5.10",
                "leverageType": "Cross",
                "liquidatedPositions": [{"coin": "BTC", "szi": "-0.125"}]
            }}),
            json!({"InternalTransfer": {
                "usdc": "1.0",
                "user": "0x22",
                "destination": "0x33",
                "fee": "0.01"
            }}),
            json!({"AccountClassTransfer": {"usdc": "1.0", "toPerp": true}}),
            json!({"SubAccountTransfer": {
                "usdc": "1.0",
                "user": "0x22",
                "destination": "0x33"
            }}),
            json!({"SpotTransfer": {
                "token": "USDC",
                "amount": "100.0",
                "usdcValue": "100.0",
                "user": "0x22",
                "destination": "0x33",
                "fee": "0.01",
                "nativeTokenFee": "0.0"
            }}),
            json!({"SpotGenesis": {"token": "USDC", "amount": "1.0"}}),
            json!({"RewardsClaim": {"amount": "1.0"}}),
            json!({"AccountActivationGas": {"amount": "1.0", "token": "USDC"}}),
            json!({"PerpDexClassTransfer": {
                "amount": "1.0",
                "token": "USDC",
                "dex": "xyz",
                "toPerp": false
            }}),
            json!({"DeployGasAuction": {"token": "USDC", "amount": "1.0"}}),
        ];
        assert_eq!(LEDGER_DELTA_VARIANTS.len(), cases.len());
        for delta in cases {
            parse_misc_event(&ledger(delta.clone()), None, 3)
                .unwrap_or_else(|_| panic!("{delta} must parse"));
        }
    }

    #[test]
    fn unknown_misc_and_ledger_variants_quarantine() {
        let inner = parse_misc_event(
            &wrap(json!({"BorrowLendRebalance": {"reserve": "USDC"}})),
            None,
            0,
        )
        .expect_err("unknown inner");
        assert!(matches!(inner, SourceError::SchemaDrift(_)));
        let delta = parse_misc_event(&ledger(json!({"MysteryDelta": {"usdc": "1"}})), None, 0)
            .expect_err("unknown delta");
        assert!(matches!(delta, SourceError::SchemaDrift(_)));
    }

    #[test]
    fn json_float_amount_is_rejected() {
        let error = parse_misc_event(
            &wrap(json!({"CDeposit": {"user": "0x22", "amount": 1.25}})),
            None,
            0,
        )
        .expect_err("float");
        assert!(matches!(error, SourceError::MalformedPayload(_)));
    }

    #[test]
    fn shipped_liquidation_and_transfer_fixtures_keep_coordinates() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures/source/node-v1");
        let liquidation = serde_json::from_slice::<Value>(
            &std::fs::read(root.join("liquidation.json")).expect("liquidation fixture"),
        )
        .expect("liquidation json")
        .as_object()
        .cloned()
        .expect("object");
        let parsed = parse_misc_event(&liquidation, Some(9), 1).expect("liquidation");
        assert_eq!(parsed.kind(), NodeRecordKind::Liquidation);
        assert_eq!(parsed.ledger_delta(), Some("Liquidation"));
        assert_eq!(parsed.coordinate().event_index(), 1);

        let transfer = serde_json::from_slice::<Value>(
            &std::fs::read(root.join("transfer.json")).expect("transfer fixture"),
        )
        .expect("transfer json")
        .as_object()
        .cloned()
        .expect("object");
        let parsed = parse_misc_event(&transfer, Some(9), 2).expect("transfer");
        assert_eq!(parsed.kind(), NodeRecordKind::Transfer);
        assert_eq!(parsed.ledger_delta(), Some("SpotTransfer"));
    }
}
