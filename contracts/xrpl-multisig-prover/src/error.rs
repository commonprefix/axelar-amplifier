use axelar_wasm_std_derive::IntoContractError;
use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq, IntoContractError)]
pub enum ContractError {
    #[error(transparent)]
    Std(#[from] StdError),

    #[error("invalid amount")]
    InvalidAmount,

    #[error("serialization failed")]
    SerializationFailed,

    #[error("invalid contract reply: {reason}")]
    InvalidContractReply { reason: String },
}

impl From<ContractError> for StdError {
    fn from(value: ContractError) -> Self {
        Self::generic_err(value.to_string())
    }
}
