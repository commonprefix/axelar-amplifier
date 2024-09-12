use std::{ops::Add, str::FromStr};

use axelar_wasm_std::{
    nonempty,
    voting::{PollId, Vote},
    Participant, Threshold,
    VerificationStatus,
};
use router_api::{Address, ChainName, CrossChainId, GatewayDirection, Message};
use xrpl_multisig_prover::querier::XRPL_CHAIN_NAME;
use xrpl_types::{msg::{XRPLHash, XRPLMessage}, types::{XRPLAccountId, XRPLToken, XRPL_MESSAGE_ID_FORMAT}};
use std::collections::{HashMap, HashSet};

use axelar_wasm_std::msg_id::HexTxHashAndEventIndex;
use coordinator::msg::ExecuteMsg as CoordinatorExecuteMsg;
use cosmwasm_std::{
    coins, Addr, Attribute, BlockInfo, Event, HexBinary, StdError, Uint128, Uint64
};
use cw_multi_test::{App, AppResponse, Executor};
use integration_tests::{contract::Contract, voting_verifier_contract::VotingContract, xrpl_gateway_contract::XRPLGatewayContract};
use integration_tests::coordinator_contract::CoordinatorContract;
use integration_tests::gateway_contract::GatewayContract;
use integration_tests::multisig_contract::MultisigContract;
use integration_tests::multisig_prover_contract::MultisigProverContract;
use integration_tests::xrpl_multisig_prover_contract::XRPLMultisigProverContract;
use integration_tests::protocol::Protocol;
use integration_tests::rewards_contract::RewardsContract;
use integration_tests::router_contract::RouterContract;
use integration_tests::service_registry_contract::ServiceRegistryContract;
use integration_tests::voting_verifier_contract::VotingVerifierContract;
use integration_tests::xrpl_voting_verifier_contract::XRPLVotingVerifierContract;
use k256::ecdsa;
use sha3::{Digest, Keccak256};

use multisig::{
    key::{KeyType, PublicKey},
    verifier_set::VerifierSet,
};
use multisig_prover::msg::VerifierSetResponse;
use rewards::state::PoolId;
use service_registry::msg::ExecuteMsg;
use tofn::ecdsa::KeyPair;

pub const AXL_DENOMINATION: &str = "uaxl";
pub const XRP_TOKEN_ID: [u8; 32] = [0; 32];
pub const ETH_TOKEN_ID: [u8; 32] = [1; 32];

pub const SIGNATURE_BLOCK_EXPIRY: u64 = 100;

pub const AXELAR_CHAIN_NAME: &str = "Axelarnet";

fn find_event_attribute<'a>(
    events: &'a [Event],
    event_type: &str,
    attribute_name: &str,
) -> Option<&'a Attribute> {
    events
        .iter()
        .find(|ev| ev.ty == event_type)?
        .attributes
        .iter()
        .find(|attribute| attribute.key == attribute_name)
}

type PollExpiryBlock = u64;

pub fn verify_messages(
    app: &mut App,
    gateway: &GatewayContract,
    msgs: &[Message],
) -> (PollId, PollExpiryBlock) {
    let response = gateway.execute(
        app,
        Addr::unchecked("relayer"),
        &gateway_api::msg::ExecuteMsg::VerifyMessages(msgs.to_vec()),
    );
    println!("response: {:?}", response);
    assert!(response.is_ok());

    let response = response.unwrap();

    let poll_id = find_event_attribute(&response.events, "wasm-messages_poll_started", "poll_id")
        .map(|attr| serde_json::from_str(&attr.value).unwrap())
        .expect("couldn't get poll_id");
    let expiry = find_event_attribute(&response.events, "wasm-messages_poll_started", "expires_at")
        .map(|attr| attr.value.as_str().parse().unwrap())
        .expect("couldn't get poll expiry");
    (poll_id, expiry)
}

pub fn verify_xrpl_messages(
    app: &mut App,
    gateway: &XRPLGatewayContract,
    msgs: &[XRPLMessage],
) -> (PollId, PollExpiryBlock) {
    let response = gateway.execute(
        app,
        Addr::unchecked("relayer"),
        &xrpl_gateway::msg::ExecuteMsg::VerifyMessages(msgs.to_vec()),
    );
    assert!(response.is_ok());

    let response = response.unwrap();

    let poll_id = find_event_attribute(&response.events, "wasm-messages_poll_started", "poll_id")
        .map(|attr| serde_json::from_str(&attr.value).unwrap())
        .expect("couldn't get poll_id");
    let expiry = find_event_attribute(&response.events, "wasm-messages_poll_started", "expires_at")
        .map(|attr| attr.value.as_str().parse().unwrap())
        .expect("couldn't get poll expiry");
    (poll_id, expiry)
}

pub fn route_messages(app: &mut App, gateway: &GatewayContract, msgs: &[Message]) {
    let response = gateway.execute(
        app,
        Addr::unchecked("relayer"),
        &gateway_api::msg::ExecuteMsg::RouteMessages(msgs.to_vec()),
    );
    println!("response: {:?}", response);
    assert!(response.is_ok());
}

pub fn freeze_chain(
    app: &mut App,
    router: &RouterContract,
    chain_name: ChainName,
    direction: GatewayDirection,
    admin: &Addr,
) {
    let response = router.execute(
        app,
        admin.clone(),
        &router_api::msg::ExecuteMsg::FreezeChains {
            chains: HashMap::from([(chain_name, direction)]),
        },
    );
    assert!(response.is_ok(), "{:?}", response);
}

pub fn unfreeze_chain(
    app: &mut App,
    router: &RouterContract,
    chain_name: &ChainName,
    direction: GatewayDirection,
    admin: &Addr,
) {
    let response = router.execute(
        app,
        admin.clone(),
        &router_api::msg::ExecuteMsg::UnfreezeChains {
            chains: HashMap::from([(chain_name.clone(), direction)]),
        },
    );
    assert!(response.is_ok(), "{:?}", response);
}

pub fn upgrade_gateway(
    app: &mut App,
    router: &RouterContract,
    governance: &Addr,
    chain_name: &ChainName,
    contract_address: Address,
) {
    let response = router.execute(
        app,
        governance.clone(),
        &router_api::msg::ExecuteMsg::UpgradeGateway {
            chain: chain_name.clone(),
            contract_address,
        },
    );
    assert!(response.is_ok(), "{:?}", response);
}

fn random_32_bytes() -> [u8; 32] {
    let mut bytes = [0; 32];
    for b in &mut bytes {
        *b = rand::random();
    }
    bytes
}

pub fn vote_success<T: VotingContract>(
    app: &mut App,
    voting_verifier: &T,
    messages_len: usize,
    verifiers: &Vec<Verifier>,
    poll_id: PollId,
)
where T::ExMsg: serde::ser::Serialize + std::fmt::Debug,
{
    for verifier in verifiers {
        let response = voting_verifier.execute(
            app,
            verifier.addr.clone(),
            &T::construct_vote_message(
                poll_id,
                messages_len,
                Vote::SucceededOnChain,
            ),
        );
        assert!(response.is_ok())
    }
}

/// Ends the poll. Be sure the current block height has advanced at least to the poll expiration, else this will fail
pub fn end_poll<T: VotingContract>(app: &mut App, voting_verifier: &T, poll_id: PollId)
where T::ExMsg: serde::ser::Serialize + std::fmt::Debug,
{
    let response = voting_verifier.execute(
        app,
        Addr::unchecked("relayer"),
        &T::construct_end_poll_message(poll_id),
    );
    assert!(response.is_ok());
}

pub fn construct_proof_and_sign(
    protocol: &mut Protocol,
    multisig_prover: &MultisigProverContract,
    messages: &[Message],
    verifiers: &Vec<Verifier>,
) -> Uint64 {
    let response = multisig_prover.execute(
        &mut protocol.app,
        Addr::unchecked("relayer"),
        &multisig_prover::msg::ExecuteMsg::ConstructProof(
            messages.iter().map(|msg| msg.cc_id.clone()).collect(),
        ),
    );
    assert!(response.is_ok());

    sign_proof(protocol, verifiers, response.unwrap())
}

pub fn multisig_session_id(response: AppResponse) -> Uint64 {
    find_event_attribute(&response.events, "wasm-signing_started", "session_id")
        .map(|attr| attr.value.as_str().try_into().unwrap())
        .expect("couldn't get session_id")
}

pub fn sign_proof(
    protocol: &mut Protocol,
    verifiers: &Vec<Verifier>,
    response: AppResponse,
) -> Uint64 {
    let msg_to_sign = find_event_attribute(&response.events, "wasm-signing_started", "msg")
        .map(|attr| attr.value.clone())
        .expect("couldn't find message to sign");
    let session_id = multisig_session_id(response);

    for verifier in verifiers {
        let signature = tofn::ecdsa::sign(
            verifier.key_pair.signing_key(),
            &HexBinary::from_hex(&msg_to_sign)
                .unwrap()
                .as_slice()
                .try_into()
                .unwrap(),
        )
        .unwrap();

        let sig = ecdsa::Signature::from_der(&signature).unwrap();

        let response = protocol.multisig.execute(
            &mut protocol.app,
            verifier.addr.clone(),
            &multisig::msg::ExecuteMsg::SubmitSignature {
                session_id,
                signature: HexBinary::from(sig.to_vec()),
            },
        );
        assert!(response.is_ok());
    }

    session_id
}

pub fn register_service(
    protocol: &mut Protocol,
    min_verifier_bond: Uint128,
    unbonding_period_days: u16,
) {
    let response = protocol.service_registry.execute(
        &mut protocol.app,
        protocol.governance_address.clone(),
        &ExecuteMsg::RegisterService {
            service_name: protocol.service_name.to_string(),
            coordinator_contract: protocol.coordinator.contract_addr.clone(),
            min_num_verifiers: 0,
            max_num_verifiers: Some(100),
            min_verifier_bond,
            bond_denom: AXL_DENOMINATION.into(),
            unbonding_period_days,
            description: "Some service".into(),
        },
    );
    assert!(response.is_ok());
}

pub fn construct_xrpl_ticket_create_proof_and_sign(
    protocol: &mut Protocol,
    multisig_prover: &XRPLMultisigProverContract,
    verifiers: &Vec<Verifier>,
) -> Uint64 {
    let response = multisig_prover.execute(
        &mut protocol.app,
        Addr::unchecked("relayer"),
        &xrpl_multisig_prover::msg::ExecuteMsg::TicketCreate,
    );
    assert!(response.is_ok());
    let response = response.unwrap();

    sign_xrpl_proof(protocol, verifiers, response)
}

pub fn construct_xrpl_payment_proof_and_sign(
    protocol: &mut Protocol,
    multisig_prover: &XRPLMultisigProverContract,
    message: Message,
    verifiers: &Vec<Verifier>,
    payload: HexBinary,
) -> Uint64 {
    let response = multisig_prover.execute(
        &mut protocol.app,
        Addr::unchecked("relayer"),
        &xrpl_multisig_prover::msg::ExecuteMsg::ConstructProof {
            message_id: message.cc_id.clone(),
            payload,
        },
    );
    assert!(response.is_ok());
    let response = response.unwrap();

    sign_xrpl_proof(protocol, verifiers, response)
}

pub fn construct_xrpl_signer_list_set_proof_and_sign(
    protocol: &mut Protocol,
    multisig_prover: &XRPLMultisigProverContract,
    verifiers: &Vec<Verifier>,
) -> Uint64 {
    let response = multisig_prover.execute(
        &mut protocol.app,
        multisig_prover.admin_addr.clone(),
        &xrpl_multisig_prover::msg::ExecuteMsg::UpdateVerifierSet,
    );
    let response = response.unwrap();

    sign_xrpl_proof(protocol, verifiers, response)
}

pub fn sign_xrpl_proof(
    protocol: &mut Protocol,
    verifiers: &Vec<Verifier>,
    response: AppResponse,
) -> Uint64 {
    let session_id: Uint64 =
        find_event_attribute(&response.events, "wasm-signing_started", "session_id")
            .map(|attr| attr.value.as_str().try_into().unwrap())
            .expect("couldn't get session_id");

    let unsigned_tx: HexBinary =
        find_event_attribute(&response.events, "wasm-xrpl_signing_started", "unsigned_tx")
            .map(|attr| HexBinary::from_hex(attr.value.as_str()).unwrap())
            .expect("couldn't get unsigned_tx");

    for verifier in verifiers {
        let xrpl_signer_address = XRPLAccountId::from(
            &multisig::key::PublicKey::Ecdsa(verifier.key_pair.encoded_verifying_key().into())
        );

        let msg = xrpl_multisig_prover::xrpl_multisig::message_to_sign(&unsigned_tx, &xrpl_signer_address).unwrap();

        let signature = tofn::ecdsa::sign(
            verifier.key_pair.signing_key(),
            &msg
                .as_slice()
                .try_into()
                .unwrap(),
        )
        .unwrap();

        let response = protocol.multisig.execute(
            &mut protocol.app,
            verifier.addr.clone(),
            &multisig::msg::ExecuteMsg::SubmitSignature {
                session_id,
                // TODO: why from_der and then back to_der?
                signature: HexBinary::from(ecdsa::Signature::from_der(&signature).unwrap().to_vec()),
            },
        );
        assert!(response.is_ok());
    }

    session_id
}

pub fn messages_from_gateway(
    app: &mut App,
    gateway: &GatewayContract,
    message_ids: &[CrossChainId],
) -> Vec<Message> {
    let query_response: Result<Vec<Message>, StdError> = gateway.query(
        app,
        &gateway_api::msg::QueryMsg::OutgoingMessages(message_ids.to_owned()),
    );
    assert!(query_response.is_ok());

    query_response.unwrap()
}

pub fn messages_from_xrpl_gateway(
    app: &mut App,
    gateway: &XRPLGatewayContract,
    message_ids: &[CrossChainId],
) -> Vec<Message> {
    let query_response: Result<Vec<Message>, StdError> = gateway.query(
        app,
        &xrpl_gateway::msg::QueryMsg::OutgoingMessages(message_ids.to_owned()),
    );
    assert!(query_response.is_ok());

    query_response.unwrap()
}

pub fn proof(
    app: &mut App,
    multisig_prover: &MultisigProverContract,
    multisig_session_id: &Uint64,
) -> multisig_prover::msg::ProofResponse {
    let query_response: Result<multisig_prover::msg::ProofResponse, StdError> = multisig_prover
        .query(
            app,
            &multisig_prover::msg::QueryMsg::Proof {
                multisig_session_id: *multisig_session_id,
            },
        );
    assert!(query_response.is_ok());

    query_response.unwrap()
}

pub fn verifier_set_from_prover(
    app: &mut App,
    multisig_prover_contract: &MultisigProverContract,
) -> VerifierSet {
    let query_response: Result<Option<VerifierSetResponse>, StdError> =
        multisig_prover_contract.query(app, &multisig_prover::msg::QueryMsg::CurrentVerifierSet);
    assert!(query_response.is_ok());

    query_response.unwrap().unwrap().verifier_set
}

pub fn get_xrpl_verifier_set_from_prover(
    app: &mut App,
    multisig_prover: &XRPLMultisigProverContract,
) -> multisig::verifier_set::VerifierSet {
    let query_response: Result<_, StdError> = multisig_prover.query(
        app,
        &xrpl_multisig_prover::msg::QueryMsg::GetVerifierSet,
    );
    assert!(query_response.is_ok());
    query_response.unwrap()
}

pub fn get_xrpl_proof(
    app: &mut App,
    multisig_prover: &XRPLMultisigProverContract,
    multisig_session_id: &Uint64,
) -> xrpl_multisig_prover::msg::GetProofResponse {
    let query_response: Result<xrpl_multisig_prover::msg::GetProofResponse, StdError> = multisig_prover.query(
        app,
        &xrpl_multisig_prover::msg::QueryMsg::GetProof {
            multisig_session_id: *multisig_session_id,
        },
    );
    assert!(query_response.is_ok());
    query_response.unwrap()
}

pub fn xrpl_update_tx_status(
    app: &mut App,
    multisig_prover: &XRPLMultisigProverContract,
    signer_public_keys: Vec<PublicKey>,
    multisig_session_id: Uint64,
    message_id: XRPLHash, // TODO: use TxHash from xrpl_multisig_prover
    message_status: VerificationStatus,
) {
    let response = multisig_prover.execute(
        app,
        Addr::unchecked("relayer"),
        &xrpl_multisig_prover::msg::ExecuteMsg::UpdateTxStatus {
            message_status,
            multisig_session_id,
            message_id: HexBinary::from(message_id).into(),
            signer_public_keys,
        },
    );
    println!("response: {:?}", response);
    assert!(response.is_ok());
}

#[allow(clippy::arithmetic_side_effects)]
pub fn advance_height(app: &mut App, increment: u64) {
    let cur_block = app.block_info();
    app.set_block(BlockInfo {
        height: cur_block.height + increment,
        ..cur_block
    });
}

pub fn advance_at_least_to_height(app: &mut App, desired_height: u64) {
    let cur_block = app.block_info();
    if app.block_info().height < desired_height {
        app.set_block(BlockInfo {
            height: desired_height,
            ..cur_block
        });
    }
}

pub fn distribute_rewards(protocol: &mut Protocol, chain_name: &ChainName, contract_address: Addr) {
    let response = protocol.rewards.execute(
        &mut protocol.app,
        Addr::unchecked("relayer"),
        &rewards::msg::ExecuteMsg::DistributeRewards {
            pool_id: PoolId {
                chain_name: chain_name.clone(),
                contract: contract_address,
            },
            epoch_count: None,
        },
    );
    assert!(response.is_ok());
}

pub fn setup_protocol(service_name: nonempty::String) -> Protocol {
    let genesis = Addr::unchecked("genesis");
    let mut app = App::new(|router, _, storage| {
        router
            .bank
            .init_balance(
                storage,
                &genesis,
                coins(u128::MAX, AXL_DENOMINATION),
            )
            .unwrap();
    });
    let admin_address = Addr::unchecked("admin");
    let governance_address = Addr::unchecked("governance");
    let nexus_gateway = Addr::unchecked("nexus_gateway");

    let router = RouterContract::instantiate_contract(
        &mut app,
        admin_address.clone(),
        governance_address.clone(),
        nexus_gateway.clone(),
    );

    let rewards_params = rewards::msg::Params {
        epoch_duration: nonempty::Uint64::try_from(10u64).unwrap(),
        rewards_per_epoch: Uint128::from(100u128).try_into().unwrap(),
        participation_threshold: (1, 2).try_into().unwrap(),
    };
    let rewards = RewardsContract::instantiate_contract(
        &mut app,
        governance_address.clone(),
        AXL_DENOMINATION.to_string(),
        rewards_params.clone(),
    );

    let multisig = MultisigContract::instantiate_contract(
        &mut app,
        governance_address.clone(),
        admin_address.clone(),
        rewards.contract_addr.clone(),
        SIGNATURE_BLOCK_EXPIRY.try_into().unwrap(),
    );

    let coordinator =
        CoordinatorContract::instantiate_contract(&mut app, governance_address.clone());

    let service_registry =
        ServiceRegistryContract::instantiate_contract(&mut app, governance_address.clone());

    Protocol {
        genesis_address: genesis,
        governance_address,
        router,
        router_admin_address: admin_address,
        multisig,
        coordinator,
        service_registry,
        service_name,
        rewards,
        rewards_params,
        app,
    }
}

// generates a key pair using the given seed. The key pair should not be used outside of testing
pub fn generate_key(seed: u32) -> KeyPair {
    let seed_bytes = seed.to_be_bytes();
    let mut result = [0; 64];
    result[0..seed_bytes.len()].copy_from_slice(seed_bytes.as_slice());
    let secret_recovery_key = result.as_slice().try_into().unwrap();
    tofn::ecdsa::keygen(&secret_recovery_key, b"tofn nonce").unwrap()
}

pub struct Verifier {
    pub addr: Addr,
    pub supported_chains: Vec<ChainName>,
    pub key_pair: KeyPair,
}

pub fn register_verifiers(
    protocol: &mut Protocol,
    verifiers: &Vec<Verifier>,
    min_verifier_bond: Uint128,
) {
    register_in_service_registry(protocol, verifiers, min_verifier_bond);
    submit_pubkeys(protocol, verifiers);
}

pub fn deregister_verifiers(protocol: &mut Protocol, verifiers: &Vec<Verifier>) {
    let response = protocol.service_registry.execute(
        &mut protocol.app,
        protocol.governance_address.clone(),
        &ExecuteMsg::UnauthorizeVerifiers {
            verifiers: verifiers
                .iter()
                .map(|verifier| verifier.addr.to_string())
                .collect(),
            service_name: protocol.service_name.to_string(),
        },
    );
    assert!(response.is_ok());

    for verifier in verifiers {
        let response = protocol.service_registry.execute(
            &mut protocol.app,
            verifier.addr.clone(),
            &ExecuteMsg::UnbondVerifier {
                service_name: protocol.service_name.to_string(),
            },
        );
        assert!(response.is_ok());
    }
}

pub fn claim_stakes(
    protocol: &mut Protocol,
    verifiers: &Vec<Verifier>,
) -> Vec<Result<AppResponse, String>> {
    let mut responses = Vec::new();

    for verifier in verifiers {
        let response = protocol.service_registry.execute(
            &mut protocol.app,
            verifier.addr.clone(),
            &ExecuteMsg::ClaimStake {
                service_name: protocol.service_name.to_string(),
            },
        );

        responses.push(response.map_err(|e| e.to_string()));
    }

    responses
}

pub fn confirm_verifier_set(
    app: &mut App,
    relayer_addr: Addr,
    multisig_prover: &MultisigProverContract,
) {
    let response = multisig_prover.execute(
        app,
        relayer_addr.clone(),
        &multisig_prover::msg::ExecuteMsg::ConfirmVerifierSet,
    );
    assert!(response.is_ok());
}

fn verifier_set_poll_id_and_expiry(response: AppResponse) -> (PollId, PollExpiryBlock) {
    let poll_id = find_event_attribute(
        &response.events,
        "wasm-verifier_set_poll_started",
        "poll_id",
    )
    .map(|attr| serde_json::from_str(&attr.value).unwrap())
    .expect("couldn't get poll_id");
    let expiry = find_event_attribute(
        &response.events,
        "wasm-verifier_set_poll_started",
        "expires_at",
    )
    .map(|attr| attr.value.as_str().parse().unwrap())
    .expect("couldn't get poll expiry");
    (poll_id, expiry)
}

pub fn create_verifier_set_poll(
    app: &mut App,
    relayer_addr: Addr,
    voting_verifier: &VotingVerifierContract,
    verifier_set: VerifierSet,
) -> (PollId, PollExpiryBlock) {
    let response = voting_verifier.execute(
        app,
        relayer_addr.clone(),
        &voting_verifier::msg::ExecuteMsg::VerifyVerifierSet {
            message_id: HexTxHashAndEventIndex {
                tx_hash: random_32_bytes(),
                event_index: 0,
            }
            .to_string()
            .parse()
            .unwrap(),
            new_verifier_set: verifier_set,
        },
    );
    assert!(response.is_ok());

    verifier_set_poll_id_and_expiry(response.unwrap())
}

pub fn verifiers_to_verifier_set(
    protocol: &mut Protocol,
    verifiers: &Vec<Verifier>,
) -> VerifierSet {
    // get public keys
    let mut pub_keys = vec![];
    for verifier in verifiers {
        let encoded_verifying_key =
            HexBinary::from(verifier.key_pair.encoded_verifying_key().to_vec());
        let pub_key = PublicKey::try_from((KeyType::Ecdsa, encoded_verifying_key)).unwrap();
        pub_keys.push(pub_key);
    }

    // turn into participants
    let participants: Vec<Participant> = verifiers
        .iter()
        .map(|verifier| Participant {
            address: verifier.addr.clone(),
            weight: Uint128::one().try_into().unwrap(),
        })
        .collect();

    let total_weight = Uint128::from(participants.len() as u128);

    let pubkeys_by_participant = participants.into_iter().zip(pub_keys).collect();

    VerifierSet::new(
        pubkeys_by_participant,
        total_weight.mul_ceil((2u64, 3u64)),
        protocol.app.block_info().height,
    )
}

// TODO: fix duplication
pub fn xrpl_verifiers_to_verifier_set(protocol: &mut Protocol, verifiers: &Vec<Verifier>) -> VerifierSet {
    // get public keys
    let mut pub_keys = vec![];
    for verifier in verifiers {
        let encoded_verifying_key =
            HexBinary::from(verifier.key_pair.encoded_verifying_key().to_vec());
        let pub_key = PublicKey::try_from((KeyType::Ecdsa, encoded_verifying_key)).unwrap();
        pub_keys.push(pub_key);
    }

    // turn into participants
    let participants: Vec<Participant> = verifiers
        .iter()
        .map(|verifier| Participant {
            address: verifier.addr.clone(),
            weight: Uint128::from(65535u128).try_into().unwrap(),
        })
        .collect();

    let total_weight = participants
        .iter()
        .fold(
            Uint128::zero(),
            |acc, p| acc.add(Uint128::from(p.weight))
        );

    let pubkeys_by_participant = participants.into_iter().zip(pub_keys).collect();

    VerifierSet::new(
        pubkeys_by_participant,
        total_weight.mul_ceil((2u64, 3u64)).into(),
        protocol.app.block_info().height,
    )
}

pub fn create_new_verifiers_vec(
    chains: Vec<ChainName>,
    verifier_details: Vec<(String, u32)>,
) -> Vec<Verifier> {
    verifier_details
        .into_iter()
        .map(|(name, seed)| Verifier {
            addr: Addr::unchecked(name),
            supported_chains: chains.clone(),
            key_pair: generate_key(seed),
        })
        .collect()
}

pub fn update_registry_and_construct_verifier_set_update_proof(
    protocol: &mut Protocol,
    new_verifiers: &Vec<Verifier>,
    verifiers_to_remove: &Vec<Verifier>,
    current_verifiers: &Vec<Verifier>,
    chain_multisig_prover: &MultisigProverContract,
    min_verifier_bond: Uint128,
) -> Uint64 {
    // Register new verifiers
    register_verifiers(protocol, new_verifiers, min_verifier_bond);

    // Deregister old verifiers
    deregister_verifiers(protocol, verifiers_to_remove);

    let response = chain_multisig_prover.execute(
        &mut protocol.app,
        Addr::unchecked("relayer"),
        &multisig_prover::msg::ExecuteMsg::UpdateVerifierSet,
    );

    sign_proof(protocol, current_verifiers, response.unwrap())
}

pub fn execute_verifier_set_poll(
    protocol: &mut Protocol,
    relayer_addr: &Addr,
    voting_verifier: &VotingVerifierContract,
    new_verifiers: &Vec<Verifier>,
) {
    // Create verifier set
    let new_verifier_set = verifiers_to_verifier_set(protocol, new_verifiers);

    // Create verifier set poll
    let (poll_id, expiry) = create_verifier_set_poll(
        &mut protocol.app,
        relayer_addr.clone(),
        voting_verifier,
        new_verifier_set.clone(),
    );

    // Vote for the verifier set
    vote_success(&mut protocol.app, voting_verifier, 1, new_verifiers, poll_id);

    // Advance to expiration height
    advance_at_least_to_height(&mut protocol.app, expiry);

    // End the poll
    end_poll(&mut protocol.app, voting_verifier, poll_id);
}

#[derive(Clone)]
pub struct Chain {
    pub gateway: GatewayContract,
    pub voting_verifier: VotingVerifierContract,
    pub multisig_prover: MultisigProverContract,
    pub its_address: Address,
    pub chain_name: ChainName,
}

#[derive(Clone)]
pub struct XRPLChain {
    pub gateway: XRPLGatewayContract,
    pub voting_verifier: XRPLVotingVerifierContract,
    pub multisig_prover: XRPLMultisigProverContract,
    pub chain_name: ChainName,
}

pub fn setup_chain(
    protocol: &mut Protocol,
    chain_name: ChainName,
    verifiers: &[Verifier],
) -> Chain {
    let voting_verifier = VotingVerifierContract::instantiate_contract(
        protocol,
        "doesn't matter".try_into().unwrap(),
        Threshold::try_from((3, 4)).unwrap().try_into().unwrap(),
        chain_name.clone(),
    );

    let gateway = GatewayContract::instantiate_contract(
        &mut protocol.app,
        protocol.router.contract_address().clone(),
        voting_verifier.contract_addr.clone(),
    );

    let multisig_prover_admin = Addr::unchecked(chain_name.to_string() + "prover_admin");
    let multisig_prover = MultisigProverContract::instantiate_contract(
        protocol,
        multisig_prover_admin.clone(),
        gateway.contract_addr.clone(),
        voting_verifier.contract_addr.clone(),
        chain_name.to_string(),
    );

    let response = multisig_prover.execute(
        &mut protocol.app,
        multisig_prover_admin,
        &multisig_prover::msg::ExecuteMsg::UpdateVerifierSet,
    );
    assert!(response.is_ok());

    let response = protocol.multisig.execute(
        &mut protocol.app,
        protocol.governance_address.clone(),
        &multisig::msg::ExecuteMsg::AuthorizeCallers {
            contracts: HashMap::from([(
                multisig_prover.contract_addr.to_string(),
                chain_name.clone(),
            )]),
        },
    );
    assert!(response.is_ok());

    let response = protocol.router.execute(
        &mut protocol.app,
        protocol.governance_address.clone(),
        &router_api::msg::ExecuteMsg::RegisterChain {
            chain: chain_name.clone(),
            gateway_address: gateway.contract_addr.to_string().try_into().unwrap(),
            msg_id_format: axelar_wasm_std::msg_id::MessageIdFormat::HexTxHashAndEventIndex,
        },
    );
    assert!(response.is_ok());

    let response = protocol.rewards.execute_with_funds(
        &mut protocol.app,
        protocol.genesis_address.clone(),
        &rewards::msg::ExecuteMsg::AddRewards {
            pool_id: PoolId {
                chain_name: chain_name.clone(),
                contract: voting_verifier.contract_addr.clone(),
            },
        },
        &coins(1000, AXL_DENOMINATION),
    );
    assert!(response.is_ok());

    let response = protocol.rewards.execute_with_funds(
        &mut protocol.app,
        protocol.genesis_address.clone(),
        &rewards::msg::ExecuteMsg::AddRewards {
            pool_id: PoolId {
                chain_name: chain_name.clone(),
                contract: protocol.multisig.contract_addr.clone(),
            },
        },
        &coins(1000, AXL_DENOMINATION),
    );
    assert!(response.is_ok());

    let response = protocol.coordinator.execute(
        &mut protocol.app,
        protocol.governance_address.clone(),
        &CoordinatorExecuteMsg::RegisterProverContract {
            chain_name: chain_name.clone(),
            new_prover_addr: multisig_prover.contract_addr.clone(),
        },
    );
    assert!(response.is_ok());

    let mut verifier_union_set = HashSet::new();
    verifier_union_set.extend(verifiers.iter().map(|verifier| verifier.addr.clone()));
    let response = protocol.coordinator.execute(
        &mut protocol.app,
        multisig_prover.contract_addr.clone(),
        &coordinator::msg::ExecuteMsg::SetActiveVerifiers {
            verifiers: verifier_union_set,
        },
    );
    assert!(response.is_ok());

    let its_address = Address::from_str("0x5CC2992f2d9ab5a74935CD1295E00bbF2CE282b2").unwrap(); // TODO
    Chain {
        gateway,
        voting_verifier,
        multisig_prover,
        chain_name,
        its_address,
    }
}

pub fn register_xrpl_token(
    protocol: &mut Protocol,
    multisig_prover: &XRPLMultisigProverContract,
    token_id: HexBinary,
    token: XRPLToken,
    decimals: u8,
) {
    let response = multisig_prover.execute(
        &mut protocol.app,
        protocol.governance_address.clone(),
        &xrpl_multisig_prover::msg::ExecuteMsg::RegisterToken {
            token_id,
            token,
            decimals,
        },
    );
    assert!(response.is_ok());
}

pub fn setup_xrpl(protocol: &mut Protocol, verifiers: &[Verifier]) -> XRPLChain {
    let chain_name = ChainName::from_str(XRPL_CHAIN_NAME).unwrap();
    let axelar_chain_name = ChainName::from_str(AXELAR_CHAIN_NAME).unwrap();

    let voting_verifier = XRPLVotingVerifierContract::instantiate_contract(
        protocol,
        "doesn't matter".to_string().try_into().unwrap(),
        Threshold::try_from((9, 10)).unwrap().try_into().unwrap(),
    );

    let axelar_its_hub_address = Addr::unchecked("its_hub"); // TODO
    let gateway= XRPLGatewayContract::instantiate_contract(
        &mut protocol.app,
        protocol.router.contract_address().clone(),
        voting_verifier.contract_addr.clone(),
        axelar_its_hub_address,
        axelar_chain_name,
    );

    let xrpl_multisig_address = "rfEf91bLxrTVC76vw1W3Ur8Jk4Lwujskmb".to_string(); // TODO: fix duplicate definition
    let multisig_prover_admin = Addr::unchecked(chain_name.to_string() + "prover_admin");
    let multisig_prover = XRPLMultisigProverContract::instantiate_contract(
        protocol,
        multisig_prover_admin.clone(),
        gateway.contract_addr.clone(),
        voting_verifier.contract_addr.clone(),
        xrpl_multisig_address.clone(),
        XRP_TOKEN_ID,
        //chain_name.to_string(),

        /*voting_verifier_address: voting_verifier_address.to_string(),
        signing_threshold: (2, 3).try_into().unwrap(),
        service_name: protocol.service_name.to_string(),
        verifier_set_diff_threshold: 1,
        xrpl_fee: 30,
        xrpl_multisig_address: xrpl_multisig_address.clone(),
        ticket_count_threshold: 1,
        next_sequence_number: 44218446,
        last_assigned_ticket_number: 44218195,
        available_tickets: vec![
            vec![],
            (44218195..44218200).collect::<Vec<_>>()
        ].concat(),
        governance_address: protocol.governance_address.to_string(),
        relayer_address: Addr::unchecked("relayer").to_string(),
        xrp_denom: "uxrp".to_string(),*/
    );

    register_xrpl_token(
        protocol,
        &multisig_prover,
        ETH_TOKEN_ID.as_slice().into(),
        XRPLToken {
            issuer: XRPLAccountId::from_str(xrpl_multisig_address.as_str()).unwrap(),
            currency: "ETH".to_string().try_into().unwrap(),
        },
        18u8,
    );

    let response = multisig_prover.execute(
        &mut protocol.app,
        multisig_prover_admin,
        &xrpl_multisig_prover::msg::ExecuteMsg::UpdateVerifierSet,
    );
    assert!(response.is_ok());

    let response = protocol.multisig.execute(
        &mut protocol.app,
        protocol.governance_address.clone(),
        &multisig::msg::ExecuteMsg::AuthorizeCallers {
            contracts: HashMap::from([(
                multisig_prover.contract_addr.to_string(),
                chain_name.clone(),
            )]),
        },
    );
    assert!(response.is_ok());

    let response = protocol.router.execute(
        &mut protocol.app,
        protocol.governance_address.clone(),
        &router_api::msg::ExecuteMsg::RegisterChain {
            chain: chain_name.clone(),
            gateway_address: gateway.contract_addr.to_string().try_into().unwrap(),
            msg_id_format: XRPL_MESSAGE_ID_FORMAT,
        },
    );
    assert!(response.is_ok());

    let response = protocol.rewards.execute_with_funds(
        &mut protocol.app,
        protocol.genesis_address.clone(),
        &rewards::msg::ExecuteMsg::AddRewards {
            pool_id: PoolId {
                chain_name: chain_name.clone(),
                contract: voting_verifier.contract_addr.clone(),
            },
        },
        &coins(1000, AXL_DENOMINATION),
    );
    assert!(response.is_ok());

    let response = protocol.rewards.execute_with_funds(
        &mut protocol.app,
        protocol.genesis_address.clone(),
        &rewards::msg::ExecuteMsg::AddRewards {
            pool_id: PoolId {
                chain_name: chain_name.clone(),
                contract: protocol.multisig.contract_addr.clone(),
            },
        },
        &coins(1000, AXL_DENOMINATION),
    );
    assert!(response.is_ok());

    let response = protocol.coordinator.execute(
        &mut protocol.app,
        protocol.governance_address.clone(),
        &CoordinatorExecuteMsg::RegisterProverContract {
            chain_name: chain_name.clone(),
            new_prover_addr: multisig_prover.contract_addr.clone(),
        },
    );
    assert!(response.is_ok());

    let mut verifier_union_set = HashSet::new();
    verifier_union_set.extend(verifiers.iter().map(|verifier| verifier.addr.clone()));
    let response = protocol.coordinator.execute(
        &mut protocol.app,
        multisig_prover.contract_addr.clone(),
        &coordinator::msg::ExecuteMsg::SetActiveVerifiers {
            verifiers: verifier_union_set,
        },
    );
    assert!(response.is_ok());

    XRPLChain {
        gateway,
        voting_verifier,
        multisig_prover,
        chain_name,
    }
}

pub fn query_balance(app: &App, address: &Addr) -> Uint128 {
    app.wrap()
        .query_balance(address, AXL_DENOMINATION)
        .unwrap()
        .amount
}

pub fn query_balances(app: &App, verifiers: &Vec<Verifier>) -> Vec<Uint128> {
    let mut balances = Vec::new();
    for verifier in verifiers {
        balances.push(query_balance(app, &verifier.addr))
    }

    balances
}

pub fn rotate_active_verifier_set(
    protocol: &mut Protocol,
    chain: Chain,
    previous_verifiers: &Vec<Verifier>,
    new_verifiers: &Vec<Verifier>,
) {
    let response = chain.multisig_prover.execute(
        &mut protocol.app,
        chain.multisig_prover.admin_addr.clone(),
        &multisig_prover::msg::ExecuteMsg::UpdateVerifierSet,
    );
    assert!(response.is_ok());

    let session_id = sign_proof(protocol, previous_verifiers, response.unwrap());

    let proof = proof(&mut protocol.app, &chain.multisig_prover, &session_id);
    assert!(matches!(
        proof.status,
        multisig_prover::msg::ProofStatus::Completed { .. }
    ));
    assert_eq!(proof.message_ids.len(), 0);

    let new_verifier_set = verifiers_to_verifier_set(protocol, new_verifiers);
    let (poll_id, expiry) = create_verifier_set_poll(
        &mut protocol.app,
        Addr::unchecked("relayer"),
        &chain.voting_verifier,
        new_verifier_set.clone(),
    );

    vote_success(
        &mut protocol.app,
        &chain.voting_verifier,
        1,
        new_verifiers,
        poll_id,
    );

    advance_at_least_to_height(&mut protocol.app, expiry);
    end_poll(&mut protocol.app, &chain.voting_verifier, poll_id);

    confirm_verifier_set(
        &mut protocol.app,
        Addr::unchecked("relayer"),
        &chain.multisig_prover,
    );
}

pub struct TestCase {
    pub protocol: Protocol,
    pub chain1: Chain,
    pub chain2: Chain,
    pub verifiers: Vec<Verifier>,
    pub min_verifier_bond: Uint128,
    pub unbonding_period_days: u16,
}

// Creates an instance of Axelar Amplifier with an initial verifier set registered, and returns a TestCase instance.
pub fn setup_test_case() -> TestCase {
    let mut protocol = setup_protocol("validators".try_into().unwrap());
    let chains = vec![
        "Ethereum".try_into().unwrap(),
        "Polygon".try_into().unwrap(),
    ];
    let verifiers = create_new_verifiers_vec(
        chains.clone(),
        vec![("verifier1".to_string(), 0), ("verifier2".to_string(), 1)],
    );

    let min_verifier_bond = Uint128::new(100);
    let unbonding_period_days = 10;
    register_service(&mut protocol, min_verifier_bond, unbonding_period_days);

    register_verifiers(&mut protocol, &verifiers, min_verifier_bond);
    let chain1 = setup_chain(&mut protocol, chains.first().unwrap().clone(), &verifiers);
    let chain2 = setup_chain(&mut protocol, chains.get(1).unwrap().clone(), &verifiers);
    TestCase {
        protocol,
        chain1,
        chain2,
        verifiers,
        min_verifier_bond,
        unbonding_period_days,
    }
}

pub fn setup_xrpl_destination_test_case() -> (Protocol, Chain, XRPLChain, Vec<Verifier>, Uint128) {
    let mut protocol = setup_protocol("validators".to_string().try_into().unwrap());
    let chains = vec![
        "Ethereum".to_string().try_into().unwrap(),
        // "Axelarnet".to_string().try_into().unwrap(),
        "XRPL".to_string().try_into().unwrap(),
    ];
    let verifiers = create_new_verifiers_vec(
        chains.clone(),
        vec![("verifier1".to_string(), 0), ("verifier2".to_string(), 1)],
    );
    let min_verifier_bond = Uint128::new(100);
    let unbonding_period_days = 10;
    register_service(&mut protocol, min_verifier_bond, unbonding_period_days);

    register_verifiers(&mut protocol, &verifiers, min_verifier_bond);
    let source_chain = setup_chain(&mut protocol, chains.first().unwrap().clone(), &verifiers);
    // let axelarnet = setup_axelarnet(&mut protocol, &verifiers);
    let xrpl = setup_xrpl(&mut protocol, &verifiers);
    println!("source_chain: {:?} {:?}", source_chain.chain_name, source_chain.gateway.contract_address());
    println!("xrpl: {:?} {:?}", xrpl.chain_name, xrpl.gateway.contract_address());
    (protocol, source_chain, xrpl, verifiers, min_verifier_bond)
}

pub fn assert_contract_err_strings_equal(
    actual: impl Into<axelar_wasm_std::error::ContractError>,
    expected: impl Into<axelar_wasm_std::error::ContractError>,
) {
    assert_eq!(actual.into().to_string(), expected.into().to_string());
}

pub fn register_in_service_registry(
    protocol: &mut Protocol,
    verifiers: &Vec<Verifier>,
    min_verifier_bond: Uint128,
) {
    let response = protocol.service_registry.execute(
        &mut protocol.app,
        protocol.governance_address.clone(),
        &ExecuteMsg::AuthorizeVerifiers {
            verifiers: verifiers
                .iter()
                .map(|verifier| verifier.addr.to_string())
                .collect(),
            service_name: protocol.service_name.to_string(),
        },
    );
    assert!(response.is_ok());

    for verifier in verifiers {
        let response = protocol.app.send_tokens(
            protocol.genesis_address.clone(),
            verifier.addr.clone(),
            &coins(min_verifier_bond.u128(), AXL_DENOMINATION),
        );
        assert!(response.is_ok());

        let response = protocol.service_registry.execute_with_funds(
            &mut protocol.app,
            verifier.addr.clone(),
            &ExecuteMsg::BondVerifier {
                service_name: protocol.service_name.to_string(),
            },
            &coins(min_verifier_bond.u128(), AXL_DENOMINATION),
        );
        assert!(response.is_ok());

        let response = protocol.service_registry.execute(
            &mut protocol.app,
            verifier.addr.clone(),
            &ExecuteMsg::RegisterChainSupport {
                service_name: protocol.service_name.to_string(),
                chains: verifier.supported_chains.clone(),
            },
        );
        assert!(response.is_ok());
    }
}

pub fn submit_pubkeys(protocol: &mut Protocol, verifiers: &Vec<Verifier>) {
    for verifier in verifiers {
        let address_hash = Keccak256::digest(verifier.addr.as_bytes());

        let sig = tofn::ecdsa::sign(
            verifier.key_pair.signing_key(),
            &address_hash.as_slice().try_into().unwrap(),
        )
        .unwrap();
        let sig = ecdsa::Signature::from_der(&sig).unwrap();

        let response = protocol.multisig.execute(
            &mut protocol.app,
            verifier.addr.clone(),
            &multisig::msg::ExecuteMsg::RegisterPublicKey {
                public_key: PublicKey::Ecdsa(HexBinary::from(
                    verifier.key_pair.encoded_verifying_key(),
                )),
                signed_sender_address: HexBinary::from(sig.to_vec()),
            },
        );
        assert!(response.is_ok());
    }
}
