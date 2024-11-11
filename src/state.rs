use crate::errors::Error;
use ethereum_ibc::client_state::ClientState;
use ethereum_ibc::consensus_state::ConsensusState;
use light_client::commitments::{gen_state_id_from_any, StateID};
use light_client::types::proto::google::protobuf::Any as IBCAny;

// canonicalize_client_state canonicalizes some fields of specified client state
// target fields: latest_execution_block_number
pub fn canonicalize_client_state<const SYNC_COMMITTEE_SIZE: usize>(
    client_state: ClientState<SYNC_COMMITTEE_SIZE>,
) -> ClientState<SYNC_COMMITTEE_SIZE> {
    let mut client_state = client_state;
    client_state.latest_execution_block_number = 0u64.into();
    client_state
}

pub fn gen_state_id<const SYNC_COMMITTEE_SIZE: usize>(
    client_state: ClientState<SYNC_COMMITTEE_SIZE>,
    consensus_state: ConsensusState,
) -> Result<StateID, Error> {
    Ok(gen_state_id_from_any(
        &IBCAny::from(canonicalize_client_state(client_state)).into(),
        &IBCAny::from(consensus_state).into(),
    )?)
}
