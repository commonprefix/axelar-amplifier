use connection_router::state::CrossChainId;
use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{to_binary, Deps, QueryRequest, StdResult, Uint64, WasmQuery, HexBinary};

use multisig::{msg::Multisig, types::MultisigState};

use crate::{
    state::{CONFIG, MULTISIG_SESSION_TX, TRANSACTION_INFO}, contract::XRPLSignedPaymentTransaction,
};

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(GetProofResponse)]
    GetProof { multisig_session_id: Uint64 },
}

#[cw_serde]
#[serde(tag = "status")]
pub enum GetProofResponse {
    Completed { message_id: CrossChainId, tx_blob: HexBinary},
    Pending { message_id: CrossChainId },
}

pub fn get_proof(deps: Deps, multisig_session_id: Uint64) -> StdResult<GetProofResponse> {
    let config = CONFIG.load(deps.storage)?;

    let tx_hash = MULTISIG_SESSION_TX.load(deps.storage, multisig_session_id.u64())?;

    let tx_info = TRANSACTION_INFO.load(deps.storage, tx_hash)?;

    let query_msg = multisig::msg::QueryMsg::GetMultisig {
        session_id: multisig_session_id,
    };

    let multisig_session: Multisig = deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: config.axelar_multisig_address.to_string(),
        msg: to_binary(&query_msg)?,
    }))?;

    let response = match multisig_session.state {
        MultisigState::Pending => GetProofResponse::Pending { message_id: tx_info.message_id },
        MultisigState::Completed { .. } => {
            let axelar_signers: Vec<(multisig::msg::Signer, multisig::key::Signature)> = multisig_session.signers
                .iter()
                .filter(|(_, signature)| signature.is_some())
                .map(|(signer, signature)| (signer.clone(), signature.clone().unwrap()))
                .collect();

            let signed_tx = XRPLSignedPaymentTransaction::new(tx_info.unsigned_contents, axelar_signers);
            let tx_blob: HexBinary = signed_tx.try_into()?;
            GetProofResponse::Completed { message_id: tx_info.message_id, tx_blob }
        }
    };

    Ok(response)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use connection_router::state::ChainName;

    use super::*;

    #[test]
    fn serialize_enum() {
        println!("{:?}", serde_json::to_string(&GetProofResponse::Completed {
            tx_blob: HexBinary::from(&[3, 1]),
            message_id: CrossChainId {
                chain: ChainName::from_str("ethereum").unwrap(),
                id: "1337".parse().unwrap()
            },
        }));
    }
}
