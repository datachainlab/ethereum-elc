use crate::errors::Error;
use crate::header::Header;
use crate::internal_prelude::*;
use crate::state::{gen_state_id, ClientState, ConsensusState};
use core::str::FromStr;
use ethereum_ibc::client_state::ClientState as EthereumClientState;
use ethereum_ibc::consensus_state::ConsensusState as EthereumConsensusState;
use ethereum_ibc::eth_client_type;
use ibc::core::ics02_client::client_state::{
    downcast_client_state, ClientState as Ics02ClientState, UpdatedState,
};
use ibc::core::ics02_client::consensus_state::{
    downcast_consensus_state, ConsensusState as Ics02ConsensusState,
};
use ibc::core::ics02_client::error::ClientError;
use ibc::core::ics02_client::header::Header as Ics02Header;
use ibc::core::ics23_commitment::commitment::{CommitmentPrefix, CommitmentProofBytes};
use ibc::core::ics24_host::Path;
use light_client::commitments::{
    CommitmentContext, StateCommitment, TrustingPeriodContext, UpdateClientCommitment,
};
use light_client::ibc::IBCContext;
use light_client::types::{Any, ClientId, Height, Time};
use light_client::{
    CreateClientResult, HostClientReader, LightClient, StateVerificationResult, UpdateClientResult,
};
use tiny_keccak::Keccak;

pub struct EthereumLightClient<const SYNC_COMMITTEE_SIZE: usize>;

impl<const SYNC_COMMITTEE_SIZE: usize> LightClient for EthereumLightClient<SYNC_COMMITTEE_SIZE> {
    fn client_type(&self) -> String {
        eth_client_type().as_str().into()
    }

    fn latest_height(
        &self,
        ctx: &dyn HostClientReader,
        client_id: &ClientId,
    ) -> Result<Height, light_client::Error> {
        let client_state: ClientState<SYNC_COMMITTEE_SIZE> =
            ctx.client_state(client_id)?.try_into()?;
        Ok(client_state.latest_height().into())
    }

    fn create_client(
        &self,
        _: &dyn HostClientReader,
        any_client_state: Any,
        any_consensus_state: Any,
    ) -> Result<light_client::CreateClientResult, light_client::Error> {
        let client_state = ClientState::<SYNC_COMMITTEE_SIZE>::try_from(any_client_state.clone())?;
        let consensus_state = ConsensusState::try_from(any_consensus_state)?;
        let _ = client_state
            .initialise(consensus_state.0.clone().into())
            .map_err(Error::ICS02)?;

        let height = client_state.latest_height().into();
        let timestamp: Time = consensus_state.timestamp().into();
        let state_id = gen_state_id(client_state, consensus_state)?;
        Ok(CreateClientResult {
            height,
            commitment: UpdateClientCommitment {
                prev_state_id: None,
                new_state_id: state_id,
                new_state: Some(any_client_state),
                prev_height: None,
                new_height: height,
                timestamp,
                context: CommitmentContext::Empty,
            }
            .into(),
            prove: false,
        })
    }

    fn update_client(
        &self,
        ctx: &dyn HostClientReader,
        client_id: ClientId,
        any_header: Any,
    ) -> Result<light_client::UpdateClientResult, light_client::Error> {
        let header = Header::<SYNC_COMMITTEE_SIZE>::try_from(any_header.clone())?;

        let client_state: ClientState<SYNC_COMMITTEE_SIZE> =
            ctx.client_state(&client_id)?.try_into()?;

        let height = header.height().into();
        let header_timestamp: Time = header.timestamp().into();
        let trusted_height = header.trusted_sync_committee.height;

        let trusted_consensus_state: ConsensusState = ctx
            .consensus_state(&client_id, &trusted_height.into())
            .map_err(|_| {
                Error::ICS02(ClientError::ConsensusStateNotFound {
                    client_id: client_id.clone().into(),
                    height: trusted_height,
                })
            })?
            .try_into()?;

        // Use client_state to validate the new header against the latest consensus_state.
        // This function will return the new client_state (its latest_height changed) and a
        // consensus_state obtained from header. These will be later persisted by the keeper.
        let UpdatedState {
            client_state: new_client_state,
            consensus_state: new_consensus_state,
        } = client_state
            .check_header_and_update_state(
                &IBCContext::<EthereumClientState<SYNC_COMMITTEE_SIZE>, EthereumConsensusState>::new(ctx),
                client_id.into(),
                any_header.into(),
            )
            .map_err(|e| {
                Error::ICS02(ClientError::HeaderVerificationFailure {
                    reason: e.to_string(),
                })
            })?;

        let new_client_state = ClientState(
            downcast_client_state::<EthereumClientState<SYNC_COMMITTEE_SIZE>>(
                new_client_state.as_ref(),
            )
            .unwrap()
            .clone(),
        );
        let new_consensus_state = ConsensusState(
            downcast_consensus_state::<EthereumConsensusState>(new_consensus_state.as_ref())
                .unwrap()
                .clone(),
        );

        let prev_state_id = gen_state_id(client_state.clone(), trusted_consensus_state.clone())?;
        let new_state_id = gen_state_id(new_client_state.clone(), new_consensus_state.clone())?;
        Ok(UpdateClientResult {
            new_any_client_state: new_client_state.into(),
            new_any_consensus_state: new_consensus_state.into(),
            height,
            commitment: UpdateClientCommitment {
                prev_state_id: Some(prev_state_id),
                new_state_id,
                new_state: None,
                prev_height: Some(trusted_height.into()),
                new_height: height,
                timestamp: header_timestamp,
                context: CommitmentContext::TrustingPeriod(TrustingPeriodContext::new(
                    client_state.trusting_period,
                    client_state.max_clock_drift,
                    header_timestamp,
                    trusted_consensus_state.timestamp.into(),
                )),
            }
            .into(),
            prove: true,
        })
    }

    fn verify_membership(
        &self,
        ctx: &dyn HostClientReader,
        client_id: ClientId,
        prefix: Vec<u8>,
        path: String,
        value: Vec<u8>,
        proof_height: Height,
        proof: Vec<u8>,
    ) -> Result<light_client::StateVerificationResult, light_client::Error> {
        let (client_state, consensus_state, prefix, path, proof) =
            Self::validate_args(ctx, client_id, prefix, path, proof_height, proof)?;

        client_state
            .verify_height(proof_height.try_into().map_err(Error::ICS02)?)
            .map_err(|e| Error::ICS02(e.into()))?;

        let value = keccak256(&value);
        client_state
            .verify_membership(
                &prefix,
                &proof,
                consensus_state.root(),
                path.clone(),
                value.to_vec(),
            )
            .map_err(|e| {
                Error::ICS02(ClientError::ClientSpecific {
                    description: e.to_string(),
                })
            })?;

        Ok(StateVerificationResult {
            state_commitment: StateCommitment::new(
                prefix.into_vec(),
                path.to_string(),
                Some(value),
                proof_height,
                gen_state_id(client_state, consensus_state)?,
            )
            .into(),
        })
    }

    fn verify_non_membership(
        &self,
        ctx: &dyn HostClientReader,
        client_id: ClientId,
        prefix: Vec<u8>,
        path: String,
        proof_height: Height,
        proof: Vec<u8>,
    ) -> Result<light_client::StateVerificationResult, light_client::Error> {
        let (client_state, consensus_state, prefix, path, proof) =
            Self::validate_args(ctx, client_id, prefix, path, proof_height, proof)?;

        client_state
            .verify_height(proof_height.try_into().map_err(Error::ICS02)?)
            .map_err(|e| Error::ICS02(e.into()))?;

        client_state
            .verify_non_membership(&prefix, &proof, consensus_state.root(), path.clone())
            .map_err(|e| {
                Error::ICS02(ClientError::ClientSpecific {
                    description: e.to_string(),
                })
            })?;

        Ok(StateVerificationResult {
            state_commitment: StateCommitment::new(
                prefix.into_vec(),
                path.to_string(),
                None,
                proof_height,
                gen_state_id(client_state, consensus_state)?,
            )
            .into(),
        })
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> EthereumLightClient<SYNC_COMMITTEE_SIZE> {
    fn validate_args(
        ctx: &dyn HostClientReader,
        client_id: ClientId,
        counterparty_prefix: Vec<u8>,
        path: String,
        proof_height: Height,
        proof: Vec<u8>,
    ) -> Result<
        (
            ClientState<SYNC_COMMITTEE_SIZE>,
            ConsensusState,
            CommitmentPrefix,
            Path,
            CommitmentProofBytes,
        ),
        light_client::Error,
    > {
        let client_state: ClientState<SYNC_COMMITTEE_SIZE> =
            ctx.client_state(&client_id)?.try_into()?;

        if client_state.is_frozen() {
            return Err(Error::ICS02(ClientError::ClientFrozen {
                client_id: client_id.into(),
            })
            .into());
        }

        let consensus_state: ConsensusState =
            ctx.consensus_state(&client_id, &proof_height)?.try_into()?;

        let proof: CommitmentProofBytes = proof.try_into().map_err(Error::ICS23)?;
        let prefix: CommitmentPrefix = counterparty_prefix.try_into().map_err(Error::ICS23)?;
        let path: Path = Path::from_str(&path).map_err(Error::ICS24Path)?;
        Ok((client_state, consensus_state, prefix, path, proof))
    }
}

fn keccak256(bz: &[u8]) -> [u8; 32] {
    let mut keccak = Keccak::new_keccak256();
    let mut result = [0u8; 32];
    keccak.update(bz);
    keccak.finalize(result.as_mut());
    result
}
