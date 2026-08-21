use domain_types::Address;

use super::{EvmLog, EvmTransaction, Hash32, Wei};

pub const TRANSFER_TOPIC: Hash32 = Hash32::from_bytes(hex_32(
    "ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
));
pub const APPROVAL_TOPIC: Hash32 = Hash32::from_bytes(hex_32(
    "8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925",
));
pub const APPROVAL_FOR_ALL_TOPIC: Hash32 = Hash32::from_bytes(hex_32(
    "17307eab39ab6107e8899845ad3d59bd9653f200f220920489ca2b5937696c31",
));
pub const TRANSFER_SINGLE_TOPIC: Hash32 = Hash32::from_bytes(hex_32(
    "c3d58168c5ae7397731d063d5bbf3d657854427343f4c083240f7aacaa2d0f62",
));
pub const TRANSFER_BATCH_TOPIC: Hash32 = Hash32::from_bytes(hex_32(
    "4a39dc06d4c0dbc64b70af90fd698a233a518aa5d07e595d983b8c0526c8f7fb",
));

const fn hex_32(hex_value: &str) -> [u8; 32] {
    let bytes = hex_value.as_bytes();
    let mut out = [0_u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (nibble(bytes[i * 2]) << 4) | nibble(bytes[i * 2 + 1]);
        i += 1;
    }
    out
}

const fn nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => 0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetStandard {
    NativeHype,
    Erc20,
    Erc721,
    Erc1155,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativeHypeTransfer {
    from: Option<Address>,
    to: Option<Address>,
    value: Wei,
}

impl NativeHypeTransfer {
    #[must_use]
    pub fn from_transaction(tx: &EvmTransaction) -> Option<Self> {
        if tx.value().is_zero() {
            return None;
        }
        Some(Self {
            from: tx.from(),
            to: tx.to(),
            value: tx.value(),
        })
    }

    #[must_use]
    pub const fn from(&self) -> Option<Address> {
        self.from
    }

    #[must_use]
    pub const fn to(&self) -> Option<Address> {
        self.to
    }

    #[must_use]
    pub const fn value(&self) -> Wei {
        self.value
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Erc20Transfer {
    contract: Address,
    from: Address,
    to: Address,
    value: Wei,
}

impl Erc20Transfer {
    #[must_use]
    pub const fn contract(&self) -> Address {
        self.contract
    }

    #[must_use]
    pub const fn from(&self) -> Address {
        self.from
    }

    #[must_use]
    pub const fn to(&self) -> Address {
        self.to
    }

    #[must_use]
    pub const fn value(&self) -> Wei {
        self.value
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Erc20Approval {
    contract: Address,
    owner: Address,
    spender: Address,
    value: Wei,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Erc721Transfer {
    contract: Address,
    from: Address,
    to: Address,
    token_id: Wei,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Erc1155BatchTransfer {
    contract: Address,
    operator: Address,
    from: Address,
    to: Address,
    ids_and_values: Vec<(Wei, Wei)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WellKnownLog {
    Erc20Transfer(Erc20Transfer),
    Erc20Approval(Erc20Approval),
    Erc721Transfer(Erc721Transfer),
    Erc1155TransferSingle {
        contract: Address,
        operator: Address,
        from: Address,
        to: Address,
        id: Wei,
        value: Wei,
    },
    Erc1155Batch(Erc1155BatchTransfer),
}

impl WellKnownLog {
    #[must_use]
    pub fn from_log(log: &EvmLog) -> Option<Self> {
        let topic0 = *log.topics().first()?;
        if topic0 == TRANSFER_TOPIC {
            if log.topics().len() == 3 && log.data().len() == 32 {
                return Some(Self::Erc20Transfer(Erc20Transfer {
                    contract: log.address(),
                    from: topic_address(log.topics()[1])?,
                    to: topic_address(log.topics()[2])?,
                    value: Wei::from_be_bytes(log.data().try_into().ok()?),
                }));
            }
            if log.topics().len() == 4 {
                return Some(Self::Erc721Transfer(Erc721Transfer {
                    contract: log.address(),
                    from: topic_address(log.topics()[1])?,
                    to: topic_address(log.topics()[2])?,
                    token_id: Wei::from_be_bytes(*log.topics()[3].as_bytes()),
                }));
            }
        }
        if topic0 == APPROVAL_TOPIC && log.topics().len() == 3 && log.data().len() == 32 {
            return Some(Self::Erc20Approval(Erc20Approval {
                contract: log.address(),
                owner: topic_address(log.topics()[1])?,
                spender: topic_address(log.topics()[2])?,
                value: Wei::from_be_bytes(log.data().try_into().ok()?),
            }));
        }
        if topic0 == TRANSFER_SINGLE_TOPIC && log.topics().len() == 4 && log.data().len() == 64 {
            let id = Wei::from_be_bytes(log.data()[..32].try_into().ok()?);
            let value = Wei::from_be_bytes(log.data()[32..].try_into().ok()?);
            return Some(Self::Erc1155TransferSingle {
                contract: log.address(),
                operator: topic_address(log.topics()[1])?,
                from: topic_address(log.topics()[2])?,
                to: topic_address(log.topics()[3])?,
                id,
                value,
            });
        }
        if topic0 == TRANSFER_BATCH_TOPIC && log.topics().len() == 4 {
            return Some(Self::Erc1155Batch(Erc1155BatchTransfer {
                contract: log.address(),
                operator: topic_address(log.topics()[1])?,
                from: topic_address(log.topics()[2])?,
                to: topic_address(log.topics()[3])?,
                // ponytail: ABI-encoded id/value arrays. T26 decodes them.
                ids_and_values: Vec::new(),
            }));
        }
        if topic0 == APPROVAL_FOR_ALL_TOPIC {
            return None;
        }
        None
    }

    #[must_use]
    pub const fn standard(&self) -> AssetStandard {
        match self {
            Self::Erc20Transfer(_) | Self::Erc20Approval(_) => AssetStandard::Erc20,
            Self::Erc721Transfer(_) => AssetStandard::Erc721,
            Self::Erc1155TransferSingle { .. } | Self::Erc1155Batch(_) => AssetStandard::Erc1155,
        }
    }
}

fn topic_address(topic: Hash32) -> Option<Address> {
    let bytes = topic.as_bytes();
    if bytes[..12].iter().any(|byte| *byte != 0) {
        return None;
    }
    Some(Address::from_bytes(bytes[12..].try_into().ok()?))
}
