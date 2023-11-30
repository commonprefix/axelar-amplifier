use axelar_wasm_std::nonempty;
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

    #[error("caller is not authorized")]
    Unauthorized,

    #[error("chain name is invalid")]
    InvalidChainName,

    #[error(transparent)]
    ServiceRegistryError(#[from] service_registry::ContractError),

    #[error(transparent)]
    NonEmptyError(#[from] nonempty::Error),

    #[error("worker set has not changed sufficiently since last update")]
    WorkerSetUnchanged,

    #[error("ticket count threshold has not been reached")]
    TicketCountThresholdNotReached,

    #[error("transaction status is already updated")]
    TransactionStatusAlreadyUpdated,

    #[error("previous ticket create transaction is pending")]
    PreviousTicketCreateTxPending,

    #[error("invalid transaction status")]
    InvalidTransactionStatus,

    #[error("transaction has not been confirmed")]
    TransactionStatusNotConfirmed,

    #[error("invalid payment amount")]
    InvalidPaymentAmount,

    #[error("invalid signing threshold")]
    InvalidSigningThreshold,

    #[error("worker set is not set")]
    WorkerSetIsNotSet,
}

impl From<ContractError> for StdError {
    fn from(value: ContractError) -> Self {
        Self::generic_err(value.to_string())
    }
}
