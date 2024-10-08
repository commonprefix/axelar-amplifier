use alloy_primitives::{FixedBytes, U256};
use alloy_sol_types::{sol, SolValue};
use axelar_wasm_std::{FnExt, IntoContractError};
use cosmwasm_std::{HexBinary, Uint256};
use error_stack::{bail, ensure, report, Report, ResultExt};
use router_api::ChainNameRaw;

use crate::primitives::{HubMessage, Message};
use crate::{TokenId, TokenManagerType};

// ITS Message payload types
// Reference: https://github.com/axelarnetwork/interchain-token-service/blob/v1.2.4/DESIGN.md#interchain-communication-spec
// `abi_encode_params` is used to encode the struct fields as ABI params as required by the spec.
// E.g. `DeployTokenManager::abi_encode_params` encodes as `abi.encode([uint256, bytes32, uint256, bytes], [...])`.
sol! {
    enum MessageType {
        InterchainTransfer,
        DeployInterchainToken,
        DeployTokenManager,
        SendToHub,
        ReceiveFromHub,
    }

    struct InterchainTransfer {
        uint256 messageType;
        bytes32 tokenId;
        bytes sourceAddress;
        bytes destinationAddress;
        uint256 amount;
        bytes data;
    }

    struct DeployInterchainToken {
        uint256 messageType;
        bytes32 tokenId;
        string name;
        string symbol;
        uint8 decimals;
        bytes minter;
    }

    struct DeployTokenManager {
        uint256 messageType;
        bytes32 tokenId;
        uint256 tokenManagerType;
        bytes params;
    }

    struct SendToHub {
        uint256 messageType;
        /// True destination chain name when sending a message from ITS edge source contract -> ITS Hub
        string destination_chain;
        bytes message;
    }

    struct ReceiveFromHub {
        uint256 messageType;
        /// True source chain name when receiving a message from ITS Hub -> ITS edge destination contract
        string source_chain;
        bytes message;
    }
}

#[derive(thiserror::Error, Debug, IntoContractError)]
pub enum Error {
    #[error("failed to decode ITS message")]
    MessageDecodeFailed,
    #[error("invalid message type")]
    InvalidMessageType,
    #[error("invalid chain name")]
    InvalidChainName,
    #[error("invalid token manager type")]
    InvalidTokenManagerType,
}

impl Message {
    pub fn abi_encode(self) -> HexBinary {
        match self {
            Message::InterchainTransfer {
                token_id,
                source_address,
                destination_address,
                amount,
                data,
            } => InterchainTransfer {
                messageType: MessageType::InterchainTransfer.into(),
                tokenId: FixedBytes::<32>::new(token_id.into()),
                sourceAddress: Vec::<u8>::from(source_address).into(),
                destinationAddress: Vec::<u8>::from(destination_address).into(),
                amount: U256::from_le_bytes(amount.to_le_bytes()),
                data: Vec::<u8>::from(data).into(),
            }
            .abi_encode_params(),
            Message::DeployInterchainToken {
                token_id,
                name,
                symbol,
                decimals,
                minter,
            } => DeployInterchainToken {
                messageType: MessageType::DeployInterchainToken.into(),
                tokenId: FixedBytes::<32>::new(token_id.into()),
                name,
                symbol,
                decimals,
                minter: Vec::<u8>::from(minter).into(),
            }
            .abi_encode_params(),
            Message::DeployTokenManager {
                token_id,
                token_manager_type,
                params,
            } => DeployTokenManager {
                messageType: MessageType::DeployTokenManager.into(),
                tokenId: FixedBytes::<32>::new(token_id.into()),
                tokenManagerType: token_manager_type.into(),
                params: Vec::<u8>::from(params).into(),
            }
            .abi_encode_params(),
        }
        .into()
    }

    pub fn abi_decode(payload: &[u8]) -> Result<Self, Report<Error>> {
        ensure!(payload.len() >= 32, Error::MessageDecodeFailed);

        let message_type = MessageType::abi_decode(&payload[0..32], true)
            .change_context(Error::InvalidMessageType)?;

        let message = match message_type {
            MessageType::InterchainTransfer => {
                let decoded = InterchainTransfer::abi_decode_params(payload, true)
                    .change_context(Error::MessageDecodeFailed)?;

                Message::InterchainTransfer {
                    token_id: TokenId::new(decoded.tokenId.into()),
                    source_address: HexBinary::from(decoded.sourceAddress.to_vec()),
                    destination_address: HexBinary::from(decoded.destinationAddress.as_ref()),
                    amount: Uint256::from_le_bytes(decoded.amount.to_le_bytes()),
                    data: HexBinary::from(decoded.data.as_ref()),
                }
            }
            MessageType::DeployInterchainToken => {
                let decoded = DeployInterchainToken::abi_decode_params(payload, true)
                    .change_context(Error::MessageDecodeFailed)?;

                Message::DeployInterchainToken {
                    token_id: TokenId::new(decoded.tokenId.into()),
                    name: decoded.name,
                    symbol: decoded.symbol,
                    decimals: decoded.decimals,
                    minter: HexBinary::from(decoded.minter.as_ref()),
                }
            }
            MessageType::DeployTokenManager => {
                let decoded = DeployTokenManager::abi_decode_params(payload, true)
                    .change_context(Error::MessageDecodeFailed)?;

                let token_manager_type = u8::try_from(decoded.tokenManagerType)
                    .change_context(Error::InvalidTokenManagerType)?
                    .then(TokenManagerType::from_repr)
                    .ok_or_else(|| report!(Error::InvalidTokenManagerType))?;

                Message::DeployTokenManager {
                    token_id: TokenId::new(decoded.tokenId.into()),
                    token_manager_type,
                    params: HexBinary::from(decoded.params.as_ref()),
                }
            }
            _ => bail!(Error::InvalidMessageType),
        };

        Ok(message)
    }
}

impl HubMessage {
    pub fn abi_encode(self) -> HexBinary {
        match self {
            HubMessage::SendToHub {
                destination_chain,
                message,
            } => SendToHub {
                messageType: MessageType::SendToHub.into(),
                destination_chain: destination_chain.into(),
                message: Vec::<u8>::from(message.abi_encode()).into(),
            }
            .abi_encode_params()
            .into(),
            HubMessage::ReceiveFromHub {
                source_chain,
                message,
            } => ReceiveFromHub {
                messageType: MessageType::ReceiveFromHub.into(),
                source_chain: source_chain.into(),
                message: Vec::<u8>::from(message.abi_encode()).into(),
            }
            .abi_encode_params()
            .into(),
        }
    }

    pub fn abi_decode(payload: &[u8]) -> Result<Self, Report<Error>> {
        ensure!(payload.len() >= 32, Error::MessageDecodeFailed);

        let message_type = MessageType::abi_decode(&payload[0..32], true)
            .change_context(Error::InvalidMessageType)?;

        let hub_message = match message_type {
            MessageType::SendToHub => {
                let decoded = SendToHub::abi_decode_params(payload, true)
                    .change_context(Error::MessageDecodeFailed)?;

                HubMessage::SendToHub {
                    destination_chain: ChainNameRaw::try_from(decoded.destination_chain)
                        .change_context(Error::InvalidChainName)?,
                    message: Message::abi_decode(&decoded.message)?,
                }
            }
            MessageType::ReceiveFromHub => {
                let decoded = ReceiveFromHub::abi_decode_params(payload, true)
                    .change_context(Error::MessageDecodeFailed)?;

                HubMessage::ReceiveFromHub {
                    source_chain: ChainNameRaw::try_from(decoded.source_chain)
                        .change_context(Error::InvalidChainName)?,
                    message: Message::abi_decode(&decoded.message)?,
                }
            }
            _ => bail!(Error::InvalidMessageType),
        };

        Ok(hub_message)
    }
}

impl From<MessageType> for U256 {
    fn from(value: MessageType) -> Self {
        U256::from(value as u8)
    }
}

impl From<TokenManagerType> for U256 {
    fn from(value: TokenManagerType) -> Self {
        U256::from(value as u8)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use alloy_primitives::{FixedBytes, U256};
    use alloy_sol_types::SolValue;
    use assert_ok::assert_ok;
    use axelar_wasm_std::assert_err_contains;
    use cosmwasm_std::{HexBinary, Uint256};
    use router_api::ChainNameRaw;

    use crate::abi::{DeployTokenManager, Error, MessageType, SendToHub};
    use crate::{HubMessage, Message, TokenManagerType};

    #[test]
    fn interchain_transfer_encode_decode() {
        let remote_chain = ChainNameRaw::from_str("chain").unwrap();

        let cases = vec![
            HubMessage::SendToHub {
                destination_chain: remote_chain.clone(),
                message: Message::InterchainTransfer {
                    token_id: [0u8; 32].into(),
                    source_address: HexBinary::from_hex("").unwrap(),
                    destination_address: HexBinary::from_hex("").unwrap(),
                    amount: Uint256::zero(),
                    data: HexBinary::from_hex("").unwrap(),
                },
            },
            HubMessage::SendToHub {
                destination_chain: remote_chain.clone(),
                message: Message::InterchainTransfer {
                    token_id: [255u8; 32].into(),
                    source_address: HexBinary::from_hex("4F4495243837681061C4743b74B3eEdf548D56A5")
                        .unwrap(),
                    destination_address: HexBinary::from_hex(
                        "4F4495243837681061C4743b74B3eEdf548D56A5",
                    )
                    .unwrap(),
                    amount: Uint256::MAX,
                    data: HexBinary::from_hex("abcd").unwrap(),
                },
            },
            HubMessage::ReceiveFromHub {
                source_chain: remote_chain.clone(),
                message: Message::InterchainTransfer {
                    token_id: [0u8; 32].into(),
                    source_address: HexBinary::from_hex("").unwrap(),
                    destination_address: HexBinary::from_hex("").unwrap(),
                    amount: Uint256::zero(),
                    data: HexBinary::from_hex("").unwrap(),
                },
            },
            HubMessage::ReceiveFromHub {
                source_chain: remote_chain.clone(),
                message: Message::InterchainTransfer {
                    token_id: [255u8; 32].into(),
                    source_address: HexBinary::from_hex("4F4495243837681061C4743b74B3eEdf548D56A5")
                        .unwrap(),
                    destination_address: HexBinary::from_hex(
                        "4F4495243837681061C4743b74B3eEdf548D56A5",
                    )
                    .unwrap(),
                    amount: Uint256::MAX,
                    data: HexBinary::from_hex("abcd").unwrap(),
                },
            },
        ];

        let encoded: Vec<_> = cases
            .iter()
            .map(|original| original.clone().abi_encode().to_hex())
            .collect();

        goldie::assert_json!(encoded);

        for original in cases {
            let encoded = original.clone().abi_encode();
            let decoded = assert_ok!(HubMessage::abi_decode(&encoded));
            assert_eq!(original, decoded);
        }
    }

    #[test]
    fn deploy_interchain_token_encode_decode() {
        let remote_chain = ChainNameRaw::from_str("chain").unwrap();

        let cases = vec![
            HubMessage::SendToHub {
                destination_chain: remote_chain.clone(),
                message: Message::DeployInterchainToken {
                    token_id: [0u8; 32].into(),
                    name: "".into(),
                    symbol: "".into(),
                    decimals: 0,
                    minter: HexBinary::from_hex("").unwrap(),
                },
            },
            HubMessage::SendToHub {
                destination_chain: remote_chain.clone(),
                message: Message::DeployInterchainToken {
                    token_id: [1u8; 32].into(),
                    name: "Test Token".into(),
                    symbol: "TST".into(),
                    decimals: 18,
                    minter: HexBinary::from_hex("1234").unwrap(),
                },
            },
            HubMessage::SendToHub {
                destination_chain: remote_chain.clone(),
                message: Message::DeployInterchainToken {
                    token_id: [0u8; 32].into(),
                    name: "Unicode Token 🪙".into(),
                    symbol: "UNI🔣".into(),
                    decimals: 255,
                    minter: HexBinary::from_hex("abcd").unwrap(),
                },
            },
            HubMessage::ReceiveFromHub {
                source_chain: remote_chain.clone(),
                message: Message::DeployInterchainToken {
                    token_id: [0u8; 32].into(),
                    name: "".into(),
                    symbol: "".into(),
                    decimals: 0,
                    minter: HexBinary::from_hex("").unwrap(),
                },
            },
            HubMessage::ReceiveFromHub {
                source_chain: remote_chain.clone(),
                message: Message::DeployInterchainToken {
                    token_id: [1u8; 32].into(),
                    name: "Test Token".into(),
                    symbol: "TST".into(),
                    decimals: 18,
                    minter: HexBinary::from_hex("1234").unwrap(),
                },
            },
            HubMessage::ReceiveFromHub {
                source_chain: remote_chain.clone(),
                message: Message::DeployInterchainToken {
                    token_id: [0u8; 32].into(),
                    name: "Unicode Token 🪙".into(),
                    symbol: "UNI🔣".into(),
                    decimals: 255,
                    minter: HexBinary::from_hex("abcd").unwrap(),
                },
            },
        ];

        let encoded: Vec<_> = cases
            .iter()
            .map(|original| original.clone().abi_encode().to_hex())
            .collect();

        goldie::assert_json!(encoded);

        for original in cases {
            let encoded = original.clone().abi_encode();
            let decoded = assert_ok!(HubMessage::abi_decode(&encoded));
            assert_eq!(original, decoded);
        }
    }

    #[test]
    fn deploy_token_manager_encode_decode() {
        let remote_chain = ChainNameRaw::from_str("chain").unwrap();

        let cases = vec![
            HubMessage::SendToHub {
                destination_chain: remote_chain.clone(),
                message: Message::DeployTokenManager {
                    token_id: [0u8; 32].into(),
                    token_manager_type: TokenManagerType::NativeInterchainToken,
                    params: HexBinary::default(),
                },
            },
            HubMessage::SendToHub {
                destination_chain: remote_chain.clone(),
                message: Message::DeployTokenManager {
                    token_id: [1u8; 32].into(),
                    token_manager_type: TokenManagerType::Gateway,
                    params: HexBinary::from_hex("1234").unwrap(),
                },
            },
            HubMessage::ReceiveFromHub {
                source_chain: remote_chain.clone(),
                message: Message::DeployTokenManager {
                    token_id: [0u8; 32].into(),
                    token_manager_type: TokenManagerType::NativeInterchainToken,
                    params: HexBinary::default(),
                },
            },
            HubMessage::ReceiveFromHub {
                source_chain: remote_chain.clone(),
                message: Message::DeployTokenManager {
                    token_id: [1u8; 32].into(),
                    token_manager_type: TokenManagerType::Gateway,
                    params: HexBinary::from_hex("1234").unwrap(),
                },
            },
        ];

        let encoded: Vec<_> = cases
            .iter()
            .map(|original| original.clone().abi_encode().to_hex())
            .collect();

        goldie::assert_json!(encoded);

        for original in cases {
            let encoded = original.clone().abi_encode();
            let decoded = assert_ok!(HubMessage::abi_decode(&encoded));
            assert_eq!(original, decoded);
        }
    }

    #[test]
    fn invalid_hub_message_type() {
        let invalid_message_types = vec![
            u8::MIN,
            MessageType::InterchainTransfer as u8,
            MessageType::DeployInterchainToken as u8,
            MessageType::DeployTokenManager as u8,
            MessageType::ReceiveFromHub as u8 + 1,
            u8::MAX,
        ];

        for message_type in invalid_message_types {
            let invalid_payload = SendToHub {
                messageType: U256::from(message_type),
                destination_chain: "remote-chain".into(),
                message: vec![].into(),
            }
            .abi_encode_params();

            let result = HubMessage::abi_decode(&invalid_payload);
            assert_err_contains!(result, Error, Error::InvalidMessageType);
        }
    }

    #[test]
    fn invalid_message_type() {
        let invalid_message_types = vec![
            MessageType::SendToHub as u8,
            MessageType::ReceiveFromHub as u8,
            MessageType::DeployTokenManager as u8 + 1,
            u8::MAX,
        ];

        for message_type in invalid_message_types {
            let invalid_payload = SendToHub {
                messageType: MessageType::SendToHub.into(),
                destination_chain: "remote-chain".into(),
                message: U256::from(message_type).abi_encode().into(),
            }
            .abi_encode_params();

            let result = HubMessage::abi_decode(&invalid_payload);
            assert_err_contains!(result, Error, Error::InvalidMessageType);
        }
    }

    #[test]
    fn invalid_destination_chain() {
        let message = DeployTokenManager {
            messageType: MessageType::DeployTokenManager.into(),
            tokenId: FixedBytes::<32>::new([0u8; 32]),
            tokenManagerType: TokenManagerType::NativeInterchainToken.into(),
            params: vec![].into(),
        };

        let payload = SendToHub {
            messageType: MessageType::SendToHub.into(),
            destination_chain: "".into(),
            message: message.abi_encode_params().into(),
        }
        .abi_encode_params();

        let result = HubMessage::abi_decode(&payload);
        assert_err_contains!(result, Error, Error::InvalidChainName);
    }

    #[test]
    fn invalid_token_manager_type() {
        let message = DeployTokenManager {
            messageType: MessageType::DeployTokenManager.into(),
            tokenId: FixedBytes::<32>::new([0u8; 32]),
            tokenManagerType: U256::from(TokenManagerType::Gateway as u8 + 1),
            params: vec![].into(),
        };

        let payload = SendToHub {
            messageType: MessageType::SendToHub.into(),
            destination_chain: "chain".into(),
            message: message.abi_encode_params().into(),
        }
        .abi_encode_params();

        let result = HubMessage::abi_decode(&payload);
        assert_err_contains!(result, Error, Error::InvalidTokenManagerType);
    }

    #[test]
    fn encode_decode_large_data() {
        let large_data = vec![0u8; 1024 * 1024]; // 1MB of data
        let original = HubMessage::SendToHub {
            destination_chain: ChainNameRaw::from_str("large-data-chain").unwrap(),
            message: Message::InterchainTransfer {
                token_id: [0u8; 32].into(),
                source_address: HexBinary::from_hex("1234").unwrap(),
                destination_address: HexBinary::from_hex("5678").unwrap(),
                amount: Uint256::from(1u128),
                data: HexBinary::from(large_data),
            },
        };

        let encoded = original.clone().abi_encode();
        let decoded = assert_ok!(HubMessage::abi_decode(&encoded));
        assert_eq!(original, decoded);
    }

    #[test]
    fn encode_decode_unicode_strings() {
        let original = HubMessage::SendToHub {
            destination_chain: ChainNameRaw::from_str("chain").unwrap(),
            message: Message::DeployInterchainToken {
                token_id: [0u8; 32].into(),
                name: "Unicode Token 🪙".into(),
                symbol: "UNI🔣".into(),
                decimals: 18,
                minter: HexBinary::from_hex("abcd").unwrap(),
            },
        };

        let encoded = original.clone().abi_encode();
        let decoded = assert_ok!(HubMessage::abi_decode(&encoded));
        assert_eq!(original, decoded);
    }
}
