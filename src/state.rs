use crate::errors::Error;
use commitments::{gen_state_id_from_any, StateID};
use core::ops::Deref;
use ethereum_ibc::client_state::{
    ClientState as EthereumClientState, ETHEREUM_CLIENT_STATE_TYPE_URL,
};
use ethereum_ibc::consensus_state::{
    ConsensusState as EthereumConsensusState, ETHEREUM_CONSENSUS_STATE_TYPE_URL,
};
use ibc_proto::google::protobuf::Any as IBCAny;
use lcp_types::Any;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ClientState<const SYNC_COMMITTEE_SIZE: usize>(
    pub(crate) EthereumClientState<SYNC_COMMITTEE_SIZE>,
);

impl<const SYNC_COMMITTEE_SIZE: usize> Deref for ClientState<SYNC_COMMITTEE_SIZE> {
    type Target = EthereumClientState<SYNC_COMMITTEE_SIZE>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> TryFrom<Any> for ClientState<SYNC_COMMITTEE_SIZE> {
    type Error = Error;

    fn try_from(value: Any) -> Result<Self, Self::Error> {
        let any: IBCAny = value.into();
        if any.type_url == ETHEREUM_CLIENT_STATE_TYPE_URL {
            Ok(Self(
                EthereumClientState::try_from(any).map_err(Error::ICS02)?,
            ))
        } else {
            Err(Error::UnexpectedClientType(any.type_url))
        }
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> From<ClientState<SYNC_COMMITTEE_SIZE>> for Any {
    fn from(value: ClientState<SYNC_COMMITTEE_SIZE>) -> Self {
        IBCAny::from(value.0).into()
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ConsensusState(pub(crate) EthereumConsensusState);

impl Deref for ConsensusState {
    type Target = EthereumConsensusState;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl TryFrom<Any> for ConsensusState {
    type Error = Error;

    fn try_from(value: Any) -> Result<Self, Self::Error> {
        let any: IBCAny = value.into();
        if any.type_url == ETHEREUM_CONSENSUS_STATE_TYPE_URL {
            Ok(Self(
                EthereumConsensusState::try_from(any).map_err(Error::ICS02)?,
            ))
        } else {
            Err(Error::UnexpectedClientType(any.type_url))
        }
    }
}

impl From<ConsensusState> for Any {
    fn from(value: ConsensusState) -> Self {
        IBCAny::from(value.0).into()
    }
}

// canonicalize_client_state canonicalizes some fields of specified client state
// target fields: latest_slot, latest_execution_block_number, frozen_height
pub fn canonicalize_client_state<const SYNC_COMMITTEE_SIZE: usize>(
    client_state: ClientState<SYNC_COMMITTEE_SIZE>,
) -> ClientState<SYNC_COMMITTEE_SIZE> {
    let mut client_state = client_state.0;
    client_state.latest_slot = 0u64.into();
    client_state.latest_execution_block_number = 0u64.into();
    client_state.frozen_height = None;
    ClientState(client_state)
}

// canonicalize_consensus_state canonicalizes some fields of specified consensus state
// target field: next_sync_committee
pub fn canonicalize_consensus_state(consensus_state: ConsensusState) -> ConsensusState {
    let mut consensus_state = consensus_state.0;
    consensus_state.next_sync_committee = None;
    ConsensusState(consensus_state)
}

pub fn gen_state_id<const SYNC_COMMITTEE_SIZE: usize>(
    client_state: ClientState<SYNC_COMMITTEE_SIZE>,
    consensus_state: ConsensusState,
) -> Result<StateID, Error> {
    Ok(gen_state_id_from_any(
        &canonicalize_client_state(client_state).try_into().unwrap(),
        &canonicalize_consensus_state(consensus_state)
            .try_into()
            .unwrap(),
    )?)
}
