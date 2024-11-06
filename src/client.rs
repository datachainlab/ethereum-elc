use crate::errors::Error;
use crate::internal_prelude::*;
use crate::state::gen_state_id;
use core::str::FromStr;
use core::time::Duration;
use ethereum_ibc::client_state::ClientState;
use ethereum_ibc::consensus_state::ConsensusState;
use ethereum_ibc::eth_client_type;
use ethereum_ibc::header::{ClientMessage, Header};
use ethereum_ibc::misbehaviour::Misbehaviour;
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
    EmittedState, MisbehaviourProxyMessage, PrevState, TrustingPeriodContext,
    UpdateStateProxyMessage, ValidationContext, VerifyMembershipProxyMessage,
};
use light_client::ibc::IBCContext;
use light_client::types::proto::google::protobuf::Any as IBCAny;
use light_client::types::{Any, ClientId, Height, Time};
use light_client::{
    CreateClientResult, HostClientReader, LightClient, MisbehaviourData, UpdateStateData,
    VerifyMembershipResult, VerifyNonMembershipResult,
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
            IBCAny::from(ctx.client_state(client_id)?)
                .try_into()
                .map_err(Error::ICS02)?;
        Ok(client_state.latest_height().into())
    }

    fn create_client(
        &self,
        _: &dyn HostClientReader,
        any_client_state: Any,
        any_consensus_state: Any,
    ) -> Result<light_client::CreateClientResult, light_client::Error> {
        let any_client_state = IBCAny::from(any_client_state);
        let any_consensus_state = IBCAny::from(any_consensus_state);
        let client_state = ClientState::<SYNC_COMMITTEE_SIZE>::try_from(any_client_state.clone())
            .map_err(Error::ICS02)?;
        let consensus_state =
            ConsensusState::try_from(any_consensus_state).map_err(Error::ICS02)?;
        let _ = client_state
            .initialise(consensus_state.clone().into())
            .map_err(Error::ICS02)?;

        let height = client_state.latest_height().into();
        let timestamp: Time = consensus_state.timestamp().into();
        let state_id = gen_state_id(client_state, consensus_state)?;
        Ok(CreateClientResult {
            height,
            message: UpdateStateProxyMessage {
                prev_height: None,
                prev_state_id: None,
                post_height: height,
                post_state_id: state_id,
                emitted_states: vec![EmittedState(height, any_client_state.into())],
                timestamp,
                context: ValidationContext::Empty,
            }
            .into(),
            prove: false,
        })
    }

    fn update_client(
        &self,
        ctx: &dyn HostClientReader,
        client_id: ClientId,
        any_message: Any,
    ) -> Result<light_client::UpdateClientResult, light_client::Error> {
        let message =
            ClientMessage::<SYNC_COMMITTEE_SIZE>::try_from(IBCAny::from(any_message.clone()))
                .map_err(Error::IBC)?;
        match message {
            ClientMessage::Header(header) => Ok(self
                .update_state(ctx, client_id, any_message, header)?
                .into()),
            ClientMessage::Misbehaviour(misbehaviour) => Ok(self
                .submit_misbehaviour(ctx, client_id, any_message, misbehaviour)?
                .into()),
        }
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
    ) -> Result<light_client::VerifyMembershipResult, light_client::Error> {
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

        Ok(VerifyMembershipResult {
            message: VerifyMembershipProxyMessage::new(
                prefix.into_vec(),
                path.to_string(),
                Some(value),
                proof_height,
                gen_state_id(client_state, consensus_state)?,
            ),
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
    ) -> Result<light_client::VerifyNonMembershipResult, light_client::Error> {
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

        Ok(VerifyNonMembershipResult {
            message: VerifyMembershipProxyMessage::new(
                prefix.into_vec(),
                path.to_string(),
                None,
                proof_height,
                gen_state_id(client_state, consensus_state)?,
            ),
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
            IBCAny::from(ctx.client_state(&client_id)?)
                .try_into()
                .map_err(Error::ICS02)?;

        if client_state.is_frozen() {
            return Err(Error::ICS02(ClientError::ClientFrozen {
                client_id: client_id.into(),
            })
            .into());
        }

        let consensus_state: ConsensusState =
            IBCAny::from(ctx.consensus_state(&client_id, &proof_height)?)
                .try_into()
                .map_err(Error::ICS02)?;

        let proof: CommitmentProofBytes = proof.try_into().map_err(Error::ICS23)?;
        let prefix: CommitmentPrefix = counterparty_prefix.try_into().map_err(Error::ICS23)?;
        let path: Path = Path::from_str(&path).map_err(Error::ICS24Path)?;
        Ok((client_state, consensus_state, prefix, path, proof))
    }

    fn update_state(
        &self,
        ctx: &dyn HostClientReader,
        client_id: ClientId,
        any_message: Any,
        header: Header<SYNC_COMMITTEE_SIZE>,
    ) -> Result<UpdateStateData, light_client::Error> {
        let client_state: ClientState<SYNC_COMMITTEE_SIZE> =
            IBCAny::from(ctx.client_state(&client_id)?)
                .try_into()
                .map_err(Error::ICS02)?;

        if client_state.is_frozen() {
            return Err(Error::ICS02(ClientError::ClientFrozen {
                client_id: client_id.into(),
            })
            .into());
        }

        let height = header.height().into();
        let header_timestamp: Time = header.timestamp().into();
        let trusted_height = header.trusted_sync_committee.height;

        let trusted_consensus_state: ConsensusState = IBCAny::from(
            ctx.consensus_state(&client_id, &trusted_height.into())
                .map_err(|_| {
                    Error::ICS02(ClientError::ConsensusStateNotFound {
                        client_id: client_id.clone().into(),
                        height: trusted_height,
                    })
                })?,
        )
        .try_into()
        .map_err(Error::ICS02)?;

        // Use client_state to validate the new header against the latest consensus_state.
        // This function will return the new client_state (its latest_height changed) and a
        // consensus_state obtained from header. These will be later persisted by the keeper.
        let UpdatedState {
            client_state: new_client_state,
            consensus_state: new_consensus_state,
        } = client_state
            .check_header_and_update_state(
                &IBCContext::<ClientState<SYNC_COMMITTEE_SIZE>, ConsensusState>::new(ctx),
                client_id.into(),
                any_message.into(),
            )
            .map_err(|e| {
                Error::ICS02(ClientError::HeaderVerificationFailure {
                    reason: e.to_string(),
                })
            })?;

        let new_client_state =
            downcast_client_state::<ClientState<SYNC_COMMITTEE_SIZE>>(new_client_state.as_ref())
                .unwrap()
                .clone();
        let new_consensus_state =
            downcast_consensus_state::<ConsensusState>(new_consensus_state.as_ref())
                .unwrap()
                .clone();

        let prev_state_id = gen_state_id(client_state.clone(), trusted_consensus_state.clone())?;
        let post_state_id = gen_state_id(new_client_state.clone(), new_consensus_state.clone())?;
        Ok(UpdateStateData {
            new_any_client_state: IBCAny::from(new_client_state).into(),
            new_any_consensus_state: IBCAny::from(new_consensus_state).into(),
            height,
            message: UpdateStateProxyMessage {
                prev_height: Some(trusted_height.into()),
                prev_state_id: Some(prev_state_id),
                post_height: height,
                post_state_id,
                emitted_states: Default::default(),
                timestamp: header_timestamp,
                context: ValidationContext::TrustingPeriod(TrustingPeriodContext::new(
                    client_state.trusting_period,
                    client_state.max_clock_drift,
                    header_timestamp,
                    trusted_consensus_state.timestamp.into(),
                )),
            },
            prove: true,
        })
    }

    fn submit_misbehaviour(
        &self,
        ctx: &dyn HostClientReader,
        client_id: ClientId,
        any_message: Any,
        misbehaviour: Misbehaviour<SYNC_COMMITTEE_SIZE>,
    ) -> Result<MisbehaviourData, light_client::Error> {
        let client_state: ClientState<SYNC_COMMITTEE_SIZE> =
            IBCAny::from(ctx.client_state(&client_id)?)
                .try_into()
                .map_err(Error::ICS02)?;

        if client_state.is_frozen() {
            return Err(Error::ICS02(ClientError::ClientFrozen {
                client_id: client_id.into(),
            })
            .into());
        }

        let trusted_height = misbehaviour.trusted_sync_committee.height;
        let trusted_consensus_state: ConsensusState = IBCAny::from(
            ctx.consensus_state(&client_id, &trusted_height.into())
                .map_err(|_| {
                    Error::ICS02(ClientError::ConsensusStateNotFound {
                        client_id: client_id.clone().into(),
                        height: trusted_height,
                    })
                })?,
        )
        .try_into()
        .map_err(Error::ICS02)?;

        let ibc_ctx = IBCContext::<ClientState<SYNC_COMMITTEE_SIZE>, ConsensusState>::new(ctx);

        let new_client_state = client_state
            .check_misbehaviour_and_update_state(
                &ibc_ctx,
                client_id.clone().into(),
                any_message.into(),
            )
            .map_err(|e| {
                Error::ICS02(ClientError::MisbehaviourHandlingFailure {
                    reason: e.to_string(),
                })
            })?;
        let new_client_state =
            downcast_client_state::<ClientState<SYNC_COMMITTEE_SIZE>>(new_client_state.as_ref())
                .unwrap()
                .clone();

        Ok(MisbehaviourData {
            new_any_client_state: IBCAny::from(new_client_state).into(),
            message: MisbehaviourProxyMessage {
                prev_states: self.make_prev_states(
                    ctx,
                    &client_id,
                    &client_state,
                    vec![trusted_height.into()],
                )?,
                // For misbehaviour, it is acceptable if the header's timestamp points to the future.
                context: ValidationContext::TrustingPeriod(TrustingPeriodContext::new(
                    client_state.trusting_period,
                    Duration::ZERO,
                    Time::unix_epoch(),
                    trusted_consensus_state.timestamp.into(),
                )),
                client_message: IBCAny::from(misbehaviour).into(),
            },
        })
    }

    fn make_prev_states(
        &self,
        ctx: &dyn HostClientReader,
        client_id: &ClientId,
        client_state: &ClientState<SYNC_COMMITTEE_SIZE>,
        heights: Vec<Height>,
    ) -> Result<Vec<PrevState>, light_client::Error> {
        let mut prev_states = Vec::new();
        for height in heights {
            let ibc_height = height.try_into().map_err(Error::ICS02)?;
            let consensus_state: ConsensusState =
                IBCAny::from(ctx.consensus_state(client_id, &height).map_err(|_| {
                    Error::ICS02(ClientError::ConsensusStateNotFound {
                        client_id: client_id.clone().into(),
                        height: ibc_height,
                    })
                })?)
                .try_into()
                .map_err(Error::ICS02)?;
            prev_states.push(PrevState {
                height,
                state_id: gen_state_id(client_state.clone(), consensus_state)?,
            });
        }
        Ok(prev_states)
    }
}

fn keccak256(bz: &[u8]) -> [u8; 32] {
    let mut keccak = Keccak::new_keccak256();
    let mut result = [0u8; 32];
    keccak.update(bz);
    keccak.finalize(result.as_mut());
    result
}
