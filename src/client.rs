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
use tiny_keccak::{Hasher, Keccak};

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
            .verify_membership(
                proof_height.try_into().map_err(Error::ICS02)?,
                &prefix,
                &proof,
                consensus_state.root(),
                path.clone(),
                value.clone(),
            )
            .map_err(|e| {
                Error::ICS02(ClientError::ClientSpecific {
                    description: format!("{:?}", e),
                })
            })?;

        Ok(VerifyMembershipResult {
            message: VerifyMembershipProxyMessage::new(
                prefix.into_vec(),
                path.to_string(),
                Some(keccak256(&value)),
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
            .verify_non_membership(
                proof_height.try_into().map_err(Error::ICS02)?,
                &prefix,
                &proof,
                consensus_state.root(),
                path.clone(),
            )
            .map_err(|e| {
                Error::ICS02(ClientError::ClientSpecific {
                    description: format!("{:?}", e),
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
                    reason: format!("{:?}", e),
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
                    reason: format!("{:?}", e),
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
    let mut hasher = Keccak::v256();
    let mut output = [0u8; 32];
    hasher.update(bz);
    hasher.finalize(&mut output);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;
    use ethereum_consensus::beacon::Version;
    use ethereum_consensus::compute::{
        compute_sync_committee_period_at_slot, compute_timestamp_at_slot,
    };
    use ethereum_consensus::context::ChainContext;
    use ethereum_consensus::fork::altair::ALTAIR_FORK_SPEC;
    use ethereum_consensus::fork::bellatrix::BELLATRIX_FORK_SPEC;
    use ethereum_consensus::fork::capella::CAPELLA_FORK_SPEC;
    use ethereum_consensus::fork::deneb::DENEB_FORK_SPEC;
    use ethereum_consensus::fork::{ForkParameter, ForkParameters};
    use ethereum_consensus::preset::minimal::PRESET;
    use ethereum_consensus::types::{Address, H256};
    use ethereum_consensus::{config, types::U64};
    use ethereum_ibc::client_state::ETHEREUM_CLIENT_REVISION_NUMBER;
    use ethereum_ibc::commitment::decode_eip1184_rlp_proof;
    use ethereum_ibc::types::{
        AccountUpdateInfo, ConsensusUpdateInfo, ExecutionUpdateInfo, TrustedSyncCommittee,
    };
    use ethereum_light_client_verifier::consensus::test_utils::{
        gen_light_client_update, gen_light_client_update_with_params,
    };
    use ethereum_light_client_verifier::context::ConsensusVerificationContext;
    use ethereum_light_client_verifier::execution::ExecutionVerifier;
    use ethereum_light_client_verifier::misbehaviour::{
        FinalizedHeaderMisbehaviour, Misbehaviour, NextSyncCommitteeMisbehaviour,
    };
    use ethereum_light_client_verifier::updates::{self, ConsensusUpdate};
    use ethereum_light_client_verifier::{
        consensus::test_utils::MockSyncCommitteeManager,
        context::{Fraction, LightClientContext},
    };
    use hex_literal::hex;
    use ibc::timestamp::Timestamp;
    use light_client::{ClientReader, HostContext, UpdateClientResult};
    use std::collections::BTreeMap;
    use std::time::SystemTime;
    use store::KVStore;

    #[derive(Debug, Clone)]
    pub struct MockContext<const SYNC_COMMITTEE_SIZE: usize> {
        pub client_state: Option<ClientState<SYNC_COMMITTEE_SIZE>>,
        pub consensus_states: BTreeMap<Height, ConsensusState>,
        pub timestamp: Time,
    }

    impl<const SYNC_COMMITTEE_SIZE: usize> MockContext<SYNC_COMMITTEE_SIZE> {
        pub fn new(timestamp: Time) -> Self {
            Self {
                client_state: None,
                consensus_states: BTreeMap::new(),
                timestamp,
            }
        }

        pub fn set_timestamp(&mut self, timestamp_secs: U64) {
            self.timestamp = Time::from_unix_timestamp(timestamp_secs.0 as i64, 0).unwrap();
        }
    }

    impl<const SYNC_COMMITTEE_SIZE: usize> HostContext for MockContext<SYNC_COMMITTEE_SIZE> {
        fn host_timestamp(&self) -> Time {
            self.timestamp
        }
    }

    impl<const SYNC_COMMITTEE_SIZE: usize> KVStore for MockContext<SYNC_COMMITTEE_SIZE> {
        fn set(&mut self, _key: Vec<u8>, _value: Vec<u8>) {
            unimplemented!()
        }

        fn get(&self, _key: &[u8]) -> Option<Vec<u8>> {
            unimplemented!()
        }

        fn remove(&mut self, _key: &[u8]) {
            unimplemented!()
        }
    }

    impl<const SYNC_COMMITTEE_SIZE: usize> ClientReader for MockContext<SYNC_COMMITTEE_SIZE> {
        fn client_state(&self, client_id: &ClientId) -> Result<Any, light_client::Error> {
            let cs = self
                .client_state
                .clone()
                .ok_or_else(|| light_client::Error::client_state_not_found(client_id.clone()))?;
            Ok(IBCAny::from(cs).into())
        }

        fn consensus_state(
            &self,
            client_id: &ClientId,
            height: &Height,
        ) -> Result<Any, light_client::Error> {
            let state = self
                .consensus_states
                .get(height)
                .ok_or_else(|| {
                    light_client::Error::consensus_state_not_found(client_id.clone(), *height)
                })?
                .clone();
            Ok(IBCAny::from(state).into())
        }
    }

    impl<const SYNC_COMMITTEE_SIZE: usize> HostClientReader for MockContext<SYNC_COMMITTEE_SIZE> {}

    #[test]
    pub fn test_client() {
        let scm = MockSyncCommitteeManager::<32>::new(1, 6);
        let lctx = LightClientContext::new_with_config(
            config::minimal::get_config(),
            keccak256("genesis_validators_root"),
            U64(1578009600),
            Fraction::new(2, 3),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .into(),
        );

        let slots_per_period = lctx.slots_per_epoch() * lctx.epochs_per_sync_committee_period();
        let base_store_period = 3u64;
        let base_store_slot = U64(base_store_period) * slots_per_period;
        let base_finalized_epoch = base_store_slot / lctx.slots_per_epoch() + 1;
        let base_attested_slot = (base_finalized_epoch + 2) * lctx.slots_per_epoch();
        let base_signature_slot = base_attested_slot + 1;

        let client_state =
            ClientState::<{ ethereum_consensus::preset::minimal::PRESET.SYNC_COMMITTEE_SIZE }> {
                genesis_validators_root: lctx.genesis_validators_root(),
                min_sync_committee_participants: 1.into(),
                genesis_time: lctx.genesis_time(),
                fork_parameters: ForkParameters::new(
                    Version([0, 0, 0, 1]),
                    vec![
                        ForkParameter::new(Version([1, 0, 0, 1]), U64(0), ALTAIR_FORK_SPEC),
                        ForkParameter::new(Version([2, 0, 0, 1]), U64(0), BELLATRIX_FORK_SPEC),
                        ForkParameter::new(Version([3, 0, 0, 1]), U64(0), CAPELLA_FORK_SPEC),
                        ForkParameter::new(Version([4, 0, 0, 1]), U64(0), DENEB_FORK_SPEC),
                    ],
                )
                .unwrap(),
                seconds_per_slot: PRESET.SECONDS_PER_SLOT,
                slots_per_epoch: PRESET.SLOTS_PER_EPOCH,
                epochs_per_sync_committee_period: PRESET.EPOCHS_PER_SYNC_COMMITTEE_PERIOD,
                ibc_address: Address(hex!("a7f733a4fEA1071f58114b203F57444969b86524")),
                ibc_commitments_slot: H256(hex!(
                    "1ee222554989dda120e26ecacf756fe1235cd8d726706b57517715dde4f0c900"
                )),
                trust_level: Fraction::new(2, 3),
                trusting_period: Duration::from_secs(60 * 60 * 27),
                max_clock_drift: Duration::from_secs(60),
                latest_execution_block_number: 1.into(),
                frozen_height: None,
                consensus_verifier: Default::default(),
                execution_verifier: Default::default(),
            };

        let consensus_state = ConsensusState {
            slot: base_store_slot,
            storage_root: hex!("27cd08827e6bf1e435832f4b2660107beb562314287b3fa534f3b189574c0cca")
                .to_vec()
                .into(),
            timestamp: Timestamp::from_nanoseconds(
                compute_timestamp_at_slot(&lctx, base_store_slot).0 * 1_000_000_000,
            )
            .unwrap(),
            current_sync_committee: scm
                .get_committee(base_store_period)
                .to_committee()
                .aggregate_pubkey,
            next_sync_committee: scm
                .get_committee(base_store_period + 1)
                .to_committee()
                .aggregate_pubkey,
        };

        let lc = EthereumLightClient::<
            { ethereum_consensus::preset::minimal::PRESET.SYNC_COMMITTEE_SIZE },
        >;

        let base_ctx = {
            let mut ctx = MockContext::<
                { ethereum_consensus::preset::minimal::PRESET.SYNC_COMMITTEE_SIZE },
            >::new(
                Time::from_unix_timestamp(lctx.genesis_time().0 as i64, 0).unwrap()
            );
            let res = lc.create_client(
                &ctx,
                IBCAny::from(client_state.clone()).into(),
                IBCAny::from(consensus_state.clone()).into(),
            );
            assert!(res.is_ok(), "{:?}", res);
            ctx.client_state = Some(client_state.clone());
            ctx.consensus_states
                .insert(res.unwrap().height, consensus_state);
            ctx
        };
        let client_id = ClientId::new(eth_client_type().as_str(), 0).unwrap();
        {
            // Update client with a header that has the same period

            let mut ctx = base_ctx.clone();
            let execution_account = get_execution_account_info();
            execution_account.validate(&ctx.client_state.as_ref().unwrap().ibc_address);
            let (consensus_update, execution_update) = gen_light_client_update::<32, _>(
                &lctx,
                base_signature_slot,
                base_attested_slot,
                base_finalized_epoch,
                execution_account.state_root,
                2.into(),
                false,
                &scm,
            );
            let header =
                Header::<{ ethereum_consensus::preset::minimal::PRESET.SYNC_COMMITTEE_SIZE }> {
                    trusted_sync_committee: get_trusted_sync_committee(
                        &lctx,
                        &ctx,
                        &scm,
                        consensus_update.signature_slot(),
                    ),
                    consensus_update: convert_consensus_update(&consensus_update),
                    execution_update: convert_execution_update(&execution_update),
                    account_update: execution_account.account_update,
                    timestamp: Timestamp::from_nanoseconds(
                        compute_timestamp_at_slot(
                            &lctx,
                            consensus_update.finalized_beacon_header().slot,
                        )
                        .0 * 1_000_000_000,
                    )
                    .unwrap(),
                };

            ctx.set_timestamp(
                compute_timestamp_at_slot(&lctx, header.consensus_update.signature_slot) + 12,
            );
            let res = lc.update_client(
                &ctx,
                client_id.clone(),
                IBCAny::from(ClientMessage::Header(header.clone())).into(),
            );
            assert!(res.is_ok(), "{:?}", res);
            let (new_client_state, new_consensus_state, height) = match res.unwrap() {
                UpdateClientResult::UpdateState(data) => (
                    data.new_any_client_state,
                    data.new_any_consensus_state,
                    data.height,
                ),
                _ => unreachable!(),
            };
            let client_state = ClientState::<32>::try_from(IBCAny::from(new_client_state)).unwrap();
            assert_eq!(client_state.latest_execution_block_number, 2.into());
            let consensus_state =
                ConsensusState::try_from(IBCAny::from(new_consensus_state)).unwrap();
            assert_eq!(
                consensus_state.slot,
                header.consensus_update.finalized_beacon_header().slot
            );
            assert_eq!(
                consensus_state.current_sync_committee,
                scm.get_committee(consensus_state.current_period(&lctx).into())
                    .to_committee()
                    .aggregate_pubkey
            );
            assert_eq!(
                consensus_state.next_sync_committee,
                scm.get_committee((consensus_state.current_period(&lctx) + 1).into())
                    .to_committee()
                    .aggregate_pubkey
            );
            ctx.client_state = Some(client_state);
            ctx.consensus_states.insert(height, consensus_state);

            // Update client with a header that has a new period

            let base_finalized_epoch =
                U64(base_store_period + 1) * slots_per_period / lctx.slots_per_epoch() + 1;
            let base_attested_slot = (base_finalized_epoch + 2) * lctx.slots_per_epoch();
            let base_signature_slot = base_attested_slot + 1;

            let execution_account = get_execution_account_info();
            execution_account.validate(&ctx.client_state.as_ref().unwrap().ibc_address);
            let (consensus_update, execution_update) = gen_light_client_update::<32, _>(
                &lctx,
                base_signature_slot,
                base_attested_slot,
                base_finalized_epoch,
                execution_account.state_root,
                3.into(),
                true,
                &scm,
            );
            let header =
                Header::<{ ethereum_consensus::preset::minimal::PRESET.SYNC_COMMITTEE_SIZE }> {
                    trusted_sync_committee: get_trusted_sync_committee(
                        &lctx,
                        &ctx,
                        &scm,
                        consensus_update.signature_slot(),
                    ),
                    consensus_update: convert_consensus_update(&consensus_update),
                    execution_update: convert_execution_update(&execution_update),
                    account_update: execution_account.account_update,
                    timestamp: Timestamp::from_nanoseconds(
                        compute_timestamp_at_slot(
                            &lctx,
                            consensus_update.finalized_beacon_header().slot,
                        )
                        .0 * 1_000_000_000,
                    )
                    .unwrap(),
                };
            ctx.set_timestamp(
                compute_timestamp_at_slot(&lctx, header.consensus_update.signature_slot) + 12,
            );
            let res = lc.update_client(
                &ctx,
                client_id.clone(),
                IBCAny::from(ClientMessage::Header(header.clone())).into(),
            );
            assert!(res.is_ok(), "{:?}", res);
            let (new_client_state, new_consensus_state, height) = match res.unwrap() {
                UpdateClientResult::UpdateState(data) => (
                    data.new_any_client_state,
                    data.new_any_consensus_state,
                    data.height,
                ),
                _ => unreachable!(),
            };
            let client_state = ClientState::<32>::try_from(IBCAny::from(new_client_state)).unwrap();
            assert_eq!(client_state.latest_execution_block_number, 3.into());
            let consensus_state =
                ConsensusState::try_from(IBCAny::from(new_consensus_state)).unwrap();
            assert_eq!(
                consensus_state.slot,
                header.consensus_update.finalized_beacon_header().slot
            );
            assert_eq!(
                consensus_state.current_sync_committee,
                scm.get_committee(consensus_state.current_period(&lctx).into())
                    .to_committee()
                    .aggregate_pubkey
            );
            assert_eq!(
                consensus_state.next_sync_committee,
                scm.get_committee((consensus_state.current_period(&lctx) + 1).into())
                    .to_committee()
                    .aggregate_pubkey
            );
            ctx.client_state = Some(client_state);
            ctx.consensus_states.insert(height, consensus_state);
        }
        // Detect FinalizedHeaderMisbehaviour
        {
            let mut ctx = base_ctx.clone();
            let (update_1, _) = gen_light_client_update_with_params::<32, _>(
                &lctx,
                base_signature_slot,
                base_attested_slot,
                base_finalized_epoch,
                [1; 32].into(),
                4.into(),
                scm.get_committee(base_store_period),
                scm.get_committee(base_store_period + 1),
                true,
                32,
            );
            let (update_2, _) = gen_light_client_update_with_params::<32, _>(
                &lctx,
                base_signature_slot,
                base_attested_slot,
                base_finalized_epoch,
                [2; 32].into(),
                4.into(),
                scm.get_committee(base_store_period),
                scm.get_committee(base_store_period + 1),
                true,
                32,
            );
            let misbehaviour = Misbehaviour::FinalizedHeader(FinalizedHeaderMisbehaviour {
                consensus_update_1: convert_consensus_update(&update_1),
                consensus_update_2: convert_consensus_update(&update_2),
            });
            ctx.set_timestamp(compute_timestamp_at_slot(&lctx, base_signature_slot) + 12);
            let res = lc.update_client(
                &ctx,
                client_id.clone(),
                IBCAny::from(ClientMessage::Misbehaviour(
                    ethereum_ibc::misbehaviour::Misbehaviour {
                        client_id: client_id.clone().into(),
                        trusted_sync_committee: get_trusted_sync_committee(
                            &lctx,
                            &ctx,
                            &scm,
                            base_signature_slot,
                        ),
                        data: misbehaviour,
                    },
                ))
                .into(),
            );
            assert!(res.is_ok(), "{:?}", res);
            let data = match res.unwrap() {
                UpdateClientResult::Misbehaviour(data) => data,
                _ => unreachable!(),
            };
            let new_client_state =
                ClientState::<32>::try_from(IBCAny::from(data.new_any_client_state.clone()))
                    .unwrap();
            assert!(new_client_state.frozen_height.is_some());
        }
        // Detect NextSyncCommitteeMisbehaviour
        {
            let mut ctx = base_ctx.clone();
            let (update_1, _) = gen_light_client_update_with_params::<32, _>(
                &lctx,
                base_signature_slot,
                base_attested_slot,
                base_finalized_epoch,
                [1; 32].into(),
                4.into(),
                scm.get_committee(base_store_period),
                scm.get_committee(base_store_period + 1),
                true,
                32,
            );
            let (update_2, _) = gen_light_client_update_with_params::<32, _>(
                &lctx,
                base_signature_slot,
                base_attested_slot,
                base_finalized_epoch,
                [1; 32].into(),
                4.into(),
                scm.get_committee(base_store_period),
                scm.get_committee(base_store_period + 2), // invalid next sync committee
                true,
                32,
            );
            let misbehaviour = Misbehaviour::NextSyncCommittee(NextSyncCommitteeMisbehaviour {
                consensus_update_1: convert_consensus_update(&update_1),
                consensus_update_2: convert_consensus_update(&update_2),
            });
            ctx.set_timestamp(compute_timestamp_at_slot(&lctx, base_signature_slot) + 12);
            let res = lc.update_client(
                &ctx,
                client_id.clone(),
                IBCAny::from(ClientMessage::Misbehaviour(
                    ethereum_ibc::misbehaviour::Misbehaviour {
                        client_id: client_id.clone().into(),
                        trusted_sync_committee: get_trusted_sync_committee(
                            &lctx,
                            &ctx,
                            &scm,
                            base_signature_slot,
                        ),
                        data: misbehaviour,
                    },
                ))
                .into(),
            );
            assert!(res.is_ok(), "{:?}", res);
            let data = match res.unwrap() {
                UpdateClientResult::Misbehaviour(data) => data,
                _ => unreachable!(),
            };
            let new_client_state =
                ClientState::<32>::try_from(IBCAny::from(data.new_any_client_state.clone()))
                    .unwrap();
            assert!(new_client_state.frozen_height.is_some());
        }
        // Verify membership of state
        {
            let mut ctx = base_ctx.clone();
            ctx.set_timestamp(compute_timestamp_at_slot(&lctx, base_signature_slot) + 12);
            let (path, proof, value) = get_membership_proof();
            let res = lc.verify_membership(
                &ctx,
                client_id.clone(),
                "ibc".as_bytes().to_vec(),
                path,
                value,
                Height::new(ETHEREUM_CLIENT_REVISION_NUMBER, 1),
                proof,
            );
            assert!(res.is_ok(), "{:?}", res);
        }
        // Verify non-membership of state
        {
            let mut ctx = base_ctx.clone();
            ctx.set_timestamp(compute_timestamp_at_slot(&lctx, base_signature_slot) + 12);
            let (path, proof) = get_non_membership_proof();
            let res = lc.verify_non_membership(
                &ctx,
                client_id.clone(),
                "ibc".as_bytes().to_vec(),
                path,
                Height::new(ETHEREUM_CLIENT_REVISION_NUMBER, 1),
                proof,
            );
            assert!(res.is_ok(), "{:?}", res);
        }
        // Verify membership of non-existing state (should fail)
        {
            let mut ctx = base_ctx.clone();
            ctx.set_timestamp(compute_timestamp_at_slot(&lctx, base_signature_slot) + 12);
            let (path, proof) = get_non_membership_proof();
            let res = lc.verify_membership(
                &ctx,
                client_id.clone(),
                "ibc".as_bytes().to_vec(),
                path,
                Default::default(),
                Height::new(ETHEREUM_CLIENT_REVISION_NUMBER, 1),
                proof,
            );
            assert!(res.is_err(), "{:?}", res);
        }
        // Verify non-membership of existing state (should fail)
        {
            let mut ctx = base_ctx.clone();
            ctx.set_timestamp(compute_timestamp_at_slot(&lctx, base_signature_slot) + 12);
            let (path, proof, _) = get_membership_proof();
            let res = lc.verify_non_membership(
                &ctx,
                client_id.clone(),
                "ibc".as_bytes().to_vec(),
                path,
                Height::new(ETHEREUM_CLIENT_REVISION_NUMBER, 1),
                proof,
            );
            assert!(res.is_err(), "{:?}", res);
        }
    }

    struct ExecutionAccountInfo {
        state_root: H256,
        account_update: AccountUpdateInfo,
    }

    impl ExecutionAccountInfo {
        pub fn validate(&self, ibc_address: &Address) {
            let valid = ExecutionVerifier
                .verify_account_storage_root(
                    self.state_root,
                    ibc_address,
                    self.account_update.account_proof.clone(),
                    self.account_update.account_storage_root,
                )
                .unwrap();
            assert!(valid, "Execution account verification failed");
        }
    }

    fn get_execution_account_info() -> ExecutionAccountInfo {
        ExecutionAccountInfo {
            state_root: H256(hex!(
                "464981a459f94d85c83a005a8ef63631bb7879a135f8b0a62cbc64582a38e65c"
            )),
            account_update: AccountUpdateInfo {
                account_proof: decode_eip1184_rlp_proof(
                    hex!("f9023ff901d1a04398e078259fe71d54d3e118b8b70126176db7a6a666049f9b0ed6bad81db6daa068be278e8fb14b2e6708c2f0484d866462468c182c1b09d9fb1259844b63cb1ca03dccdb9c963884c8c0b36ff9f770c0b4ba702df8647c7555d64afc3657808953a0d14aef547f57a65e31e6756b1012fd2df269dceec0a45be53c02273b208929efa09c8d1c426dc97706b5c561f610b6d0c9289d7e77ea1a3324955684b919d742d480a0bfe090c221fa27abcc7f0b02bd0aecf246e3f1f620d33d98ee5463c621726b8080a031a80c76669969c6b2b75250c772b7e6bd96c74c9b4c9ab7f2b6aa082d310f41a0a6cd8a58c4c7642f25924ab0ea9c952aec850fd0410463fa3dbc9917b0c3e1e8a07753f936bbbf1f0430deea77beeedc3f77bddc8c863cdea1dce4b4f254c2659ba0a3a4bf85e7f3bf2a1a5385e2249356b83108162ad17bf2a8cf77ab57dbc7503da0376dba981a2ca2c1173c6d3bdf04b80e7446f8b1e964fefa4f0175e9f94e9d81a0ae11a26d017ccf8f7a9b0d4aff4639a24f038bf1ee5e10e41bf7aba4f45c7036a00b29d0a90e5f768e4bf4eb4b9efa8cd1ad41798d6bb08ccef06d53ac3f8e1a79a0c0850eb680dc0893f352571a86dc94f6eddf134f2c0a3f2737bb045a3f373eea80f869a03b51e6579cd86645444e321a80ba4795cbd7cda9712dae2aefba7cdbcf962d8eb846f8440180a027cd08827e6bf1e435832f4b2660107beb562314287b3fa534f3b189574c0ccaa0cf9747f04c15d40e52400ba1082794772b0c6a7bb8b554e52c6af3e19281186c").to_vec()
                ).unwrap(),
                account_storage_root: H256(hex!(
                    "27cd08827e6bf1e435832f4b2660107beb562314287b3fa534f3b189574c0cca"
                )),
            }
        }
    }

    // returns: (path, proof, value)
    fn get_membership_proof() -> (String, Vec<u8>, Vec<u8>) {
        (
            "clients/lcp-client-0/clientState".to_string(),
            hex!("f90159f901118080a0143145e818eeff83817419a6632ea193fd1acaa4f791eb17282f623f38117f56a0e6ee0a993a7254ee9253d766ea005aec74eb1e11656961f0fb11323f4f91075580808080a01efae04adc2e970b4af3517581f41ce2ba4ff60492d33696c1e2a5ab70cb55bba03bac3f5124774e41fb6efdd7219530846f9f6441045c4666d2855c6598cfca00a020d7122ffc86cb37228940b5a9441e9fd272a3450245c9130ca3ab00bc1cd6ef80a0047f255205a0f2b0e7d29d490abf02bfb62c3ed201c338bc7f0088fa9c5d77eda069fecc766fcb2df04eb3a834b1f4ba134df2be114479e251d9cc9b6ba493077b80a094c3ed6a7ef63a6a67e46cc9876b9b1882eeba3d28e6d61bb15cdfb207d077e180f843a03e077f3dfd0489e70c68282ced0126c62fcef50acdcb7f57aa4552b87b456b11a1a05dc044e92e82db28c96fd98edd502949612b06e8da6dd74664a43a5ed857b298").to_vec(),
            hex!("0a242f6962632e6c69676874636c69656e74732e6c63702e76312e436c69656e74537461746512ed010a208083673c69fe3f098ea79a799d9dbb99c39b4b4f17a1a79ef58bdf8ae86299951080f524220310fb012a1353575f48415244454e494e475f4e45454445442a1147524f55505f4f55545f4f465f44415445320e494e54454c2d53412d3030323139320e494e54454c2d53412d3030323839320e494e54454c2d53412d3030333334320e494e54454c2d53412d3030343737320e494e54454c2d53412d3030363134320e494e54454c2d53412d3030363135320e494e54454c2d53412d3030363137320e494e54454c2d53412d30303832383a14cb96f8d6c2d543102184d679d7829b39434e4eec48015001").to_vec()
        )
    }

    // returns: (path, proof)
    fn get_non_membership_proof() -> (String, Vec<u8>) {
        (
            "clients/lcp-client-1/clientState".to_string(),
            hex!("f90114f901118080a0143145e818eeff83817419a6632ea193fd1acaa4f791eb17282f623f38117f56a0e6ee0a993a7254ee9253d766ea005aec74eb1e11656961f0fb11323f4f91075580808080a01efae04adc2e970b4af3517581f41ce2ba4ff60492d33696c1e2a5ab70cb55bba03bac3f5124774e41fb6efdd7219530846f9f6441045c4666d2855c6598cfca00a020d7122ffc86cb37228940b5a9441e9fd272a3450245c9130ca3ab00bc1cd6ef80a0047f255205a0f2b0e7d29d490abf02bfb62c3ed201c338bc7f0088fa9c5d77eda069fecc766fcb2df04eb3a834b1f4ba134df2be114479e251d9cc9b6ba493077b80a094c3ed6a7ef63a6a67e46cc9876b9b1882eeba3d28e6d61bb15cdfb207d077e180").to_vec()
        )
    }

    fn get_trusted_sync_committee<const SYNC_COMMITTEE_SIZE: usize>(
        lctx: &LightClientContext,
        ctx: &MockContext<SYNC_COMMITTEE_SIZE>,
        scm: &MockSyncCommitteeManager<SYNC_COMMITTEE_SIZE>,
        signature_slot: U64,
    ) -> TrustedSyncCommittee<SYNC_COMMITTEE_SIZE> {
        let client_state = ctx.client_state.as_ref().unwrap();
        let consensus_state = ctx
            .consensus_states
            .get(&client_state.latest_height().into())
            .unwrap();
        let store_period = consensus_state.current_period(lctx);
        let target_period = compute_sync_committee_period_at_slot(lctx, signature_slot);
        if store_period == target_period {
            TrustedSyncCommittee {
                height: client_state.latest_height(),
                sync_committee: scm.get_committee(target_period.into()).to_committee(),
                is_next: false,
            }
        } else if store_period + 1 == target_period {
            TrustedSyncCommittee {
                height: client_state.latest_height(),
                sync_committee: scm.get_committee(target_period.into()).to_committee(),
                is_next: true,
            }
        } else {
            panic!(
                "Invalid target: target_period={} store_period={} signature_slot={}",
                target_period, store_period, signature_slot
            );
        }
    }

    fn convert_consensus_update<const SYNC_COMMITTEE_SIZE: usize>(
        consensus_update: &updates::ConsensusUpdateInfo<SYNC_COMMITTEE_SIZE>,
    ) -> ConsensusUpdateInfo<SYNC_COMMITTEE_SIZE> {
        ConsensusUpdateInfo {
            attested_header: consensus_update.light_client_update.attested_header.clone(),
            next_sync_committee: consensus_update
                .light_client_update
                .next_sync_committee
                .clone(),
            finalized_header: (
                consensus_update.finalized_beacon_header().clone(),
                consensus_update.finalized_beacon_header_branch(),
            ),
            sync_aggregate: consensus_update.sync_aggregate().clone(),
            signature_slot: consensus_update.signature_slot(),
            finalized_execution_root: consensus_update.finalized_execution_root(),
            finalized_execution_branch: consensus_update.finalized_execution_branch(),
        }
    }

    fn convert_execution_update(
        execution_update: &updates::ExecutionUpdateInfo,
    ) -> ExecutionUpdateInfo {
        ExecutionUpdateInfo {
            state_root: execution_update.state_root,
            state_root_branch: execution_update.state_root_branch.clone(),
            block_number: execution_update.block_number,
            block_number_branch: execution_update.block_number_branch.clone(),
        }
    }

    fn keccak256(s: &str) -> H256 {
        use tiny_keccak::{Hasher, Keccak};
        let mut hasher = Keccak::v256();
        let mut output = [0u8; 32];
        hasher.update(s.as_bytes());
        hasher.finalize(&mut output);
        H256::from_slice(&output)
    }
}
