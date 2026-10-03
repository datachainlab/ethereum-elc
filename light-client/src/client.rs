use crate::client_state::{ClientState, ETHEREUM_CLIENT_REVISION_NUMBER};
use crate::consensus_state::ConsensusState;
use crate::errors::Error;
use crate::header::{ClientMessage, Header};
use crate::internal_prelude::*;
use crate::misbehaviour::Misbehaviour;
use crate::misc::{to_lc_types_height, to_lcp_height};
use crate::state::gen_state_id;
use core::time::Duration;
use ethereum_light_client_types::client_state::ClientState as EthClientState;
use ethereum_light_client_types::membership::{verify_membership, verify_non_membership};
use light_client::commitments::{
    EmittedState, MisbehaviourProxyMessage, PrevState, TrustingPeriodContext,
    UpdateStateProxyMessage, ValidationContext, VerifyMembershipProxyMessage,
};
use light_client::types::proto::google::protobuf::Any as IBCAny;
use light_client::types::{Any, ClientId, Height, Time};
use light_client::{
    CreateClientResult, HostClientReader, LightClient, MisbehaviourData, UpdateStateData,
    VerifyMembershipResult, VerifyNonMembershipResult,
};

pub struct EthereumLightClient<const SYNC_COMMITTEE_SIZE: usize>;

pub(crate) const ETHEREUM_CLIENT_TYPE: &str = "ethereum";

impl<const SYNC_COMMITTEE_SIZE: usize> LightClient for EthereumLightClient<SYNC_COMMITTEE_SIZE> {
    fn client_type(&self) -> String {
        ETHEREUM_CLIENT_TYPE.into()
    }

    fn latest_height(
        &self,
        ctx: &dyn HostClientReader,
        client_id: &ClientId,
    ) -> Result<Height, light_client::Error> {
        let any_client_state = ctx.client_state(client_id)?;
        let client_state = ClientState::<SYNC_COMMITTEE_SIZE>::try_from(any_client_state)?;
        Ok(to_lcp_height(client_state.latest_height()))
    }

    fn create_client(
        &self,
        _: &dyn HostClientReader,
        any_client_state: Any,
        any_consensus_state: Any,
    ) -> Result<CreateClientResult, light_client::Error> {
        let client_state = ClientState::<SYNC_COMMITTEE_SIZE>::try_from(any_client_state.clone())?;
        client_state.validate()?;
        if client_state.is_frozen() {
            return Err(Error::CannotInitializeFrozenClient.into());
        }
        let consensus_state = ConsensusState::try_from(any_consensus_state)?;
        consensus_state.validate()?;

        let height = to_lcp_height(client_state.latest_height());
        let timestamp = consensus_state.timestamp;
        let state_id = gen_state_id(client_state, consensus_state)?;
        Ok(CreateClientResult {
            height,
            message: UpdateStateProxyMessage {
                prev_height: None,
                prev_state_id: None,
                post_height: height,
                post_state_id: state_id,
                emitted_states: vec![EmittedState(height, any_client_state)],
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
            ClientMessage::<SYNC_COMMITTEE_SIZE>::try_from(IBCAny::from(any_message.clone()))?;
        match message {
            ClientMessage::Header(header) => Ok(self.update_state(ctx, client_id, header)?.into()),
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
    ) -> Result<VerifyMembershipResult, light_client::Error> {
        let any_client_state = ctx.client_state(&client_id)?;
        let any_consensus_state = ctx.consensus_state(&client_id, &proof_height)?;
        let client_state = ClientState::<SYNC_COMMITTEE_SIZE>::try_from(any_client_state.clone())?;
        if client_state.is_frozen() {
            return Err(Error::ClientFrozen(client_id).into());
        }
        let consensus_state = ConsensusState::try_from(any_consensus_state)?;
        let value = verify_membership(
            &client_state,
            &consensus_state,
            path.clone(),
            value,
            to_lc_types_height(proof_height),
            proof,
            &client_state.execution_verifier,
        )
        .map_err(Error::EthereumLightClientTypes)?;
        Ok(VerifyMembershipResult {
            message: VerifyMembershipProxyMessage::new(
                prefix.to_vec(),
                path,
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
    ) -> Result<VerifyNonMembershipResult, light_client::Error> {
        let any_client_state = ctx.client_state(&client_id)?;
        let any_consensus_state = ctx.consensus_state(&client_id, &proof_height)?;
        let client_state = ClientState::<SYNC_COMMITTEE_SIZE>::try_from(any_client_state.clone())?;
        if client_state.is_frozen() {
            return Err(Error::ClientFrozen(client_id).into());
        }
        let consensus_state = ConsensusState::try_from(any_consensus_state)?;
        verify_non_membership(
            &client_state,
            &consensus_state,
            path.clone(),
            to_lc_types_height(proof_height),
            proof,
            &client_state.execution_verifier,
        )
        .map_err(Error::EthereumLightClientTypes)?;
        Ok(VerifyNonMembershipResult {
            message: VerifyMembershipProxyMessage::new(
                prefix.to_vec(),
                path,
                None,
                proof_height,
                gen_state_id(client_state, consensus_state)?,
            ),
        })
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> EthereumLightClient<SYNC_COMMITTEE_SIZE> {
    fn update_state(
        &self,
        ctx: &dyn HostClientReader,
        client_id: ClientId,
        header: Header<SYNC_COMMITTEE_SIZE>,
    ) -> Result<UpdateStateData, light_client::Error> {
        let height = Height::new(
            ETHEREUM_CLIENT_REVISION_NUMBER,
            header.execution_update.block_number.0,
        );
        let trusted_height = to_lcp_height(header.trusted_sync_committee.height);

        let any_client_state = ctx.client_state(&client_id)?;
        let any_consensus_state = ctx.consensus_state(&client_id, &trusted_height)?;

        // Ensure client is not frozen
        let client_state = ClientState::<SYNC_COMMITTEE_SIZE>::try_from(any_client_state)?;
        if client_state.is_frozen() {
            return Err(Error::ClientFrozen(client_id).into());
        }

        // Create new state and ensure header is valid
        let consensus_state = ConsensusState::try_from(any_consensus_state)?;

        let (new_client_state, new_consensus_state) = client_state.check_header_and_update_state(
            ctx.host_timestamp(),
            &consensus_state,
            header,
        )?;

        let header_timestamp = new_consensus_state.timestamp;
        let prev_state_id = gen_state_id(client_state.clone(), consensus_state.clone())?;
        let post_state_id = gen_state_id(new_client_state.clone(), new_consensus_state.clone())?;
        Ok(UpdateStateData {
            new_any_client_state: new_client_state.try_into()?,
            new_any_consensus_state: new_consensus_state.try_into()?,
            height,
            message: UpdateStateProxyMessage {
                prev_height: Some(trusted_height),
                prev_state_id: Some(prev_state_id),
                post_height: height,
                post_state_id,
                emitted_states: Default::default(),
                timestamp: header_timestamp,
                context: ValidationContext::TrustingPeriod(TrustingPeriodContext::new(
                    client_state.trusting_period,
                    client_state.max_clock_drift,
                    header_timestamp,
                    consensus_state.timestamp,
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
        let trusted_height = to_lcp_height(misbehaviour.trusted_sync_committee.height);
        let any_client_state = ctx.client_state(&client_id)?;
        let any_consensus_state = ctx.consensus_state(&client_id, &trusted_height)?;
        // Ensure client is not frozen
        let client_state = ClientState::<SYNC_COMMITTEE_SIZE>::try_from(any_client_state)?;
        if client_state.is_frozen() {
            return Err(Error::ClientFrozen(client_id).into());
        }

        // Create new state and ensure header is valid
        let consensus_state = ConsensusState::try_from(any_consensus_state)?;

        let new_client_state = client_state.check_misbehaviour_and_update_state(
            ctx.host_timestamp(),
            &client_id,
            &consensus_state,
            misbehaviour,
        )?;

        Ok(MisbehaviourData {
            new_any_client_state: new_client_state.try_into()?,
            message: MisbehaviourProxyMessage {
                prev_states: self.make_prev_states(
                    ctx,
                    &client_id,
                    &client_state,
                    vec![trusted_height],
                )?,
                // For misbehaviour, it is acceptable if the header's timestamp points to the future.
                context: ValidationContext::TrustingPeriod(TrustingPeriodContext::new(
                    client_state.trusting_period,
                    Duration::ZERO,
                    Time::unix_epoch(),
                    consensus_state.timestamp,
                )),
                client_message: any_message,
            },
        })
    }

    fn make_prev_states(
        &self,
        ctx: &dyn HostClientReader,
        client_id: &ClientId,
        client_state: &ClientState<SYNC_COMMITTEE_SIZE>,
        heights: Vec<Height>,
    ) -> Result<Vec<PrevState>, Error> {
        let mut prev_states = Vec::new();
        for height in heights {
            let consensus_state: ConsensusState =
                ctx.consensus_state(client_id, &height)?.try_into()?;
            prev_states.push(PrevState {
                height,
                state_id: gen_state_id(client_state.clone(), consensus_state)?,
            });
        }
        Ok(prev_states)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::Header;
    use crate::misbehaviour::Misbehaviour;
    use crate::misc::new_timestamp;
    use crate::test_utils::{
        account_proof, create_test_client_state_from_ctx, to_consensus_update_info, TestFixture,
        SYNC_COMMITTEE_SIZE,
    };
    use ethereum_consensus::compute::compute_timestamp_at_slot;
    use ethereum_consensus::types::H256;
    use ethereum_light_client_types::consensus::{AccountUpdateInfo, ExecutionUpdateInfo};
    use ethereum_light_client_verifier::misbehaviour::{
        FinalizedHeaderMisbehaviour, Misbehaviour as MisbehaviourData,
    };
    use ethereum_light_client_verifier::updates::ConsensusUpdate;
    use light_client::{ClientKeeper, ClientReader, HostContext};
    use store::KVStore;

    /// Mock implementation of HostClientReader for testing using store
    struct MockHostContext {
        store: store::memory::MemStore,
        host_timestamp: Time,
    }

    impl MockHostContext {
        fn new(host_timestamp: Time) -> Self {
            Self {
                store: store::memory::MemStore::default(),
                host_timestamp,
            }
        }

        /// Store client state and consensus state for testing
        fn setup_client(
            &mut self,
            client_id: &ClientId,
            client_state: Any,
            consensus_state: Any,
            height: Height,
        ) {
            self.store_client_type(client_id.clone(), ETHEREUM_CLIENT_TYPE.to_string())
                .unwrap();
            self.store_any_client_state(client_id.clone(), client_state)
                .unwrap();
            self.store_any_consensus_state(client_id.clone(), height, consensus_state)
                .unwrap();
        }
    }

    impl KVStore for MockHostContext {
        fn set(&mut self, key: Vec<u8>, value: Vec<u8>) {
            self.store.set(key, value);
        }

        fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
            self.store.get(key)
        }

        fn remove(&mut self, key: &[u8]) {
            self.store.remove(key);
        }
    }

    impl HostContext for MockHostContext {
        fn host_timestamp(&self) -> Time {
            self.host_timestamp
        }
    }

    impl ClientReader for MockHostContext {}
    impl ClientKeeper for MockHostContext {}
    impl HostClientReader for MockHostContext {}

    fn create_test_consensus_state(fixture: &TestFixture, timestamp: u64) -> ConsensusState {
        fixture.consensus_state(
            fixture.period_1,
            account_proof::get_storage_root(),
            new_timestamp(timestamp).unwrap(),
        )
    }

    #[test]
    fn test_client_type() {
        let client = EthereumLightClient::<SYNC_COMMITTEE_SIZE>;
        assert_eq!(client.client_type(), ETHEREUM_CLIENT_TYPE);
    }

    #[test]
    fn test_latest_height() {
        let fixture = TestFixture::new();
        let mut client_state = create_test_client_state_from_ctx(&fixture.ctx);
        client_state.latest_execution_block_number = 12345u64.into();

        let consensus_state = create_test_consensus_state(&fixture, 1577836800 + 1000);

        let client_id = ClientId::new(ETHEREUM_CLIENT_TYPE, 0).unwrap();
        let any_client_state: Any = client_state.try_into().unwrap();
        let any_consensus_state: Any = consensus_state.try_into().unwrap();

        let mut mock_ctx = MockHostContext::new(Time::unix_epoch());
        mock_ctx.setup_client(
            &client_id,
            any_client_state,
            any_consensus_state,
            Height::new(0, 1),
        );

        let client = EthereumLightClient::<SYNC_COMMITTEE_SIZE>;
        let height = client.latest_height(&mock_ctx, &client_id).unwrap();

        assert_eq!(height.revision_number(), ETHEREUM_CLIENT_REVISION_NUMBER);
        assert_eq!(height.revision_height(), 12345);
    }

    #[test]
    fn test_latest_height_client_not_found() {
        let client_id = ClientId::new(ETHEREUM_CLIENT_TYPE, 999).unwrap();
        let mock_ctx = MockHostContext::new(Time::unix_epoch());

        let client = EthereumLightClient::<SYNC_COMMITTEE_SIZE>;
        let result = client.latest_height(&mock_ctx, &client_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_client_success() {
        let fixture = TestFixture::new();
        let client_state = create_test_client_state_from_ctx(&fixture.ctx);
        let consensus_state = create_test_consensus_state(&fixture, 1577836800 + 1000);

        let any_client_state: Any = client_state.clone().try_into().unwrap();
        let any_consensus_state: Any = consensus_state.try_into().unwrap();

        let mock_ctx = MockHostContext::new(Time::unix_epoch());
        let client = EthereumLightClient::<SYNC_COMMITTEE_SIZE>;

        let result = client.create_client(&mock_ctx, any_client_state, any_consensus_state);
        assert!(result.is_ok(), "create_client failed: {:?}", result);

        let create_result = result.unwrap();
        assert_eq!(
            create_result.height.revision_height(),
            client_state.latest_execution_block_number.0
        );
        assert!(!create_result.prove);
    }

    #[test]
    fn test_create_client_frozen_client_fails() {
        let fixture = TestFixture::new();
        let mut client_state = create_test_client_state_from_ctx(&fixture.ctx);
        client_state.frozen_height = Some(Height::new(0, 100));

        let consensus_state = create_test_consensus_state(&fixture, 1577836800 + 1000);

        let any_client_state: Any = client_state.try_into().unwrap();
        let any_consensus_state: Any = consensus_state.try_into().unwrap();

        let mock_ctx = MockHostContext::new(Time::unix_epoch());
        let client = EthereumLightClient::<SYNC_COMMITTEE_SIZE>;

        let result = client.create_client(&mock_ctx, any_client_state, any_consensus_state);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_client_invalid_client_state_fails() {
        let client_state = ClientState::<SYNC_COMMITTEE_SIZE>::default();
        let consensus_state = ConsensusState::default();

        let any_client_state: Any = client_state.try_into().unwrap();
        let any_consensus_state: Any = consensus_state.try_into().unwrap();

        let mock_ctx = MockHostContext::new(Time::unix_epoch());
        let client = EthereumLightClient::<SYNC_COMMITTEE_SIZE>;

        let result = client.create_client(&mock_ctx, any_client_state, any_consensus_state);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_client_unknown_client_state_type_fails() {
        let any_client_state = Any::new("/unknown.type".to_string(), vec![]);
        let any_consensus_state: Any = ConsensusState::default().try_into().unwrap();

        let mock_ctx = MockHostContext::new(Time::unix_epoch());
        let client = EthereumLightClient::<SYNC_COMMITTEE_SIZE>;

        let result = client.create_client(&mock_ctx, any_client_state, any_consensus_state);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_client_invalid_consensus_state_fails() {
        let fixture = TestFixture::new();
        let client_state = create_test_client_state_from_ctx(&fixture.ctx);

        let any_client_state: Any = client_state.try_into().unwrap();
        let any_consensus_state = Any::new("/unknown.consensus".to_string(), vec![]);

        let mock_ctx = MockHostContext::new(Time::unix_epoch());
        let client = EthereumLightClient::<SYNC_COMMITTEE_SIZE>;

        let result = client.create_client(&mock_ctx, any_client_state, any_consensus_state);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_membership_frozen_client_fails() {
        let fixture = TestFixture::new();
        let mut client_state = create_test_client_state_from_ctx(&fixture.ctx);
        client_state.frozen_height = Some(Height::new(0, 50));

        let consensus_state = create_test_consensus_state(&fixture, 1577836800 + 1000);

        let client_id = ClientId::new(ETHEREUM_CLIENT_TYPE, 0).unwrap();
        let proof_height = Height::new(0, 1);

        let any_client_state: Any = client_state.try_into().unwrap();
        let any_consensus_state: Any = consensus_state.try_into().unwrap();

        let mut mock_ctx = MockHostContext::new(Time::unix_epoch());
        mock_ctx.setup_client(
            &client_id,
            any_client_state,
            any_consensus_state,
            proof_height,
        );

        let client = EthereumLightClient::<SYNC_COMMITTEE_SIZE>;
        let result = client.verify_membership(
            &mock_ctx,
            client_id,
            vec![],
            "some/path".to_string(),
            vec![1, 2, 3],
            proof_height,
            vec![],
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_non_membership_frozen_client_fails() {
        let fixture = TestFixture::new();
        let mut client_state = create_test_client_state_from_ctx(&fixture.ctx);
        client_state.frozen_height = Some(Height::new(0, 50));

        let consensus_state = create_test_consensus_state(&fixture, 1577836800 + 1000);

        let client_id = ClientId::new(ETHEREUM_CLIENT_TYPE, 0).unwrap();
        let proof_height = Height::new(0, 1);

        let any_client_state: Any = client_state.try_into().unwrap();
        let any_consensus_state: Any = consensus_state.try_into().unwrap();

        let mut mock_ctx = MockHostContext::new(Time::unix_epoch());
        mock_ctx.setup_client(
            &client_id,
            any_client_state,
            any_consensus_state,
            proof_height,
        );

        let client = EthereumLightClient::<SYNC_COMMITTEE_SIZE>;
        let result = client.verify_non_membership(
            &mock_ctx,
            client_id,
            vec![],
            "some/path".to_string(),
            proof_height,
            vec![],
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_update_state_frozen_client_fails() {
        let fixture = TestFixture::new();
        let execution_state_root = account_proof::get_state_root();
        let dummy_execution_block_number = 100u64;

        let (update, execution_update) =
            fixture.gen_update(execution_state_root, dummy_execution_block_number);
        let update_info = to_consensus_update_info(update);
        let finalized_slot = update_info.finalized_beacon_header().slot;
        let timestamp_secs = compute_timestamp_at_slot(&fixture.ctx, finalized_slot).0;

        let consensus_state_slot = fixture.period_1 + 1;
        let consensus_state = fixture.consensus_state(
            consensus_state_slot,
            account_proof::get_storage_root(),
            new_timestamp(timestamp_secs - 1000).unwrap(),
        );

        let execution_update_info = ExecutionUpdateInfo {
            state_root: execution_update.state_root,
            state_root_branch: execution_update.state_root_branch,
            block_number: execution_update.block_number,
            block_number_branch: execution_update.block_number_branch,
            rlp: vec![],
            block_hash: H256::default(),
            block_hash_branch: vec![],
        };

        let header = Header {
            trusted_sync_committee: fixture.trusted_from_current(Height::new(0, 1), false),
            consensus_update: update_info,
            execution_update: execution_update_info,
            account_update: AccountUpdateInfo {
                account_proof: account_proof::get_proof(),
                account_storage_root: account_proof::get_storage_root(),
            },
        };

        let mut client_state = create_test_client_state_from_ctx(&fixture.ctx);
        client_state.ibc_address = account_proof::get_address();
        client_state.frozen_height = Some(Height::new(0, 50)); // Frozen

        let client_id = ClientId::new(ETHEREUM_CLIENT_TYPE, 0).unwrap();
        let any_client_state: Any = client_state.try_into().unwrap();
        let any_consensus_state: Any = consensus_state.try_into().unwrap();

        let now = new_timestamp(timestamp_secs + 100).unwrap();
        let mut mock_ctx = MockHostContext::new(now);
        mock_ctx.setup_client(
            &client_id,
            any_client_state,
            any_consensus_state,
            Height::new(0, 1),
        );

        let client = EthereumLightClient::<SYNC_COMMITTEE_SIZE>;
        let result = client.update_state(&mock_ctx, client_id, header);
        assert!(result.is_err());
    }

    #[test]
    fn test_update_state_success() {
        let fixture = TestFixture::new();
        let execution_state_root = account_proof::get_state_root();
        let dummy_execution_block_number = 100u64;

        let (update, execution_update) =
            fixture.gen_update(execution_state_root, dummy_execution_block_number);
        let update_info = to_consensus_update_info(update);
        let finalized_slot = update_info.finalized_beacon_header().slot;
        let timestamp_secs = compute_timestamp_at_slot(&fixture.ctx, finalized_slot).0;

        let consensus_state_slot = fixture.period_1 + 1;
        let consensus_state = fixture.consensus_state(
            consensus_state_slot,
            account_proof::get_storage_root(),
            new_timestamp(timestamp_secs - 1000).unwrap(),
        );

        let execution_update_info = ExecutionUpdateInfo {
            state_root: execution_update.state_root,
            state_root_branch: execution_update.state_root_branch,
            block_number: execution_update.block_number,
            block_number_branch: execution_update.block_number_branch,
            rlp: vec![],
            block_hash: H256::default(),
            block_hash_branch: vec![],
        };

        let header = Header {
            trusted_sync_committee: fixture.trusted_from_current(Height::new(0, 1), false),
            consensus_update: update_info,
            execution_update: execution_update_info,
            account_update: AccountUpdateInfo {
                account_proof: account_proof::get_proof(),
                account_storage_root: account_proof::get_storage_root(),
            },
        };

        let mut client_state = create_test_client_state_from_ctx(&fixture.ctx);
        client_state.ibc_address = account_proof::get_address();

        let client_id = ClientId::new(ETHEREUM_CLIENT_TYPE, 0).unwrap();
        let any_client_state: Any = client_state.try_into().unwrap();
        let any_consensus_state: Any = consensus_state.try_into().unwrap();

        let now = new_timestamp(timestamp_secs + 100).unwrap();
        let mut mock_ctx = MockHostContext::new(now);
        mock_ctx.setup_client(
            &client_id,
            any_client_state,
            any_consensus_state,
            Height::new(0, 1),
        );

        let client = EthereumLightClient::<SYNC_COMMITTEE_SIZE>;
        let result = client.update_state(&mock_ctx, client_id, header);
        assert!(result.is_ok(), "update_state failed: {:?}", result);

        let update_data = result.unwrap();
        assert_eq!(
            update_data.height.revision_height(),
            dummy_execution_block_number
        );
        assert!(update_data.prove);
    }

    // ========================================================================
    // Tests for submit_misbehaviour
    // ========================================================================

    #[test]
    fn test_submit_misbehaviour_frozen_client_fails() {
        let fixture = TestFixture::new();

        let (update_1, _) = fixture.gen_update([1u8; 32].into(), 100);
        let (update_2, _) = fixture.gen_update([2u8; 32].into(), 100);

        let update_info_1 = to_consensus_update_info(update_1);
        let update_info_2 = to_consensus_update_info(update_2);
        let finalized_slot = update_info_1.finalized_beacon_header().slot;
        let timestamp_secs = compute_timestamp_at_slot(&fixture.ctx, finalized_slot).0;

        let consensus_state = fixture.consensus_state(
            fixture.period_1 + 1,
            [1u8; 32].into(),
            new_timestamp(timestamp_secs - 1000).unwrap(),
        );

        let trusted_height = Height::new(0, 1);
        let misbehaviour = Misbehaviour {
            client_id: ClientId::new(ETHEREUM_CLIENT_TYPE, 0).unwrap(),
            trusted_sync_committee: fixture.trusted_from_current(trusted_height, false),
            data: MisbehaviourData::FinalizedHeader(FinalizedHeaderMisbehaviour {
                consensus_update_1: update_info_1,
                consensus_update_2: update_info_2,
            }),
        };

        // Create already frozen client state
        let mut client_state = create_test_client_state_from_ctx(&fixture.ctx);
        client_state.frozen_height = Some(Height::new(0, 50)); // Already frozen

        let client_id = ClientId::new(ETHEREUM_CLIENT_TYPE, 0).unwrap();
        let any_client_state: Any = client_state.try_into().unwrap();
        let any_consensus_state: Any = consensus_state.try_into().unwrap();

        let now = new_timestamp(timestamp_secs + 100).unwrap();
        let mut mock_ctx = MockHostContext::new(now);
        mock_ctx.setup_client(
            &client_id,
            any_client_state,
            any_consensus_state,
            trusted_height,
        );

        let any_message = Any::new("dummy".to_string(), vec![]);
        let client = EthereumLightClient::<SYNC_COMMITTEE_SIZE>;
        let result = client.submit_misbehaviour(&mock_ctx, client_id, any_message, misbehaviour);

        assert!(result.is_err());
    }

    #[test]
    fn test_submit_misbehaviour_success() {
        let fixture = TestFixture::new();

        // Create two updates with different execution state roots at the same finalized slot
        let (update_1, _) = fixture.gen_update([1u8; 32].into(), 100);
        let (update_2, _) = fixture.gen_update([2u8; 32].into(), 100);

        let update_info_1 = to_consensus_update_info(update_1);
        let update_info_2 = to_consensus_update_info(update_2);
        let finalized_slot = update_info_1.finalized_beacon_header().slot;
        let timestamp_secs = compute_timestamp_at_slot(&fixture.ctx, finalized_slot).0;

        let consensus_state = fixture.consensus_state(
            fixture.period_1 + 1,
            [1u8; 32].into(),
            new_timestamp(timestamp_secs - 1000).unwrap(),
        );

        let trusted_height = Height::new(0, 1);
        let client_id = ClientId::new(ETHEREUM_CLIENT_TYPE, 0).unwrap();
        let misbehaviour = Misbehaviour {
            client_id: client_id.clone(),
            trusted_sync_committee: fixture.trusted_from_current(trusted_height, false),
            data: MisbehaviourData::FinalizedHeader(FinalizedHeaderMisbehaviour {
                consensus_update_1: update_info_1,
                consensus_update_2: update_info_2,
            }),
        };

        let client_state = create_test_client_state_from_ctx(&fixture.ctx);
        assert!(!client_state.is_frozen());

        let any_client_state: Any = client_state.try_into().unwrap();
        let any_consensus_state: Any = consensus_state.try_into().unwrap();

        let now = new_timestamp(timestamp_secs + 100).unwrap();
        let mut mock_ctx = MockHostContext::new(now);
        mock_ctx.setup_client(
            &client_id,
            any_client_state,
            any_consensus_state,
            trusted_height,
        );

        let any_message = Any::new("dummy".to_string(), vec![]);
        let client = EthereumLightClient::<SYNC_COMMITTEE_SIZE>;
        let result = client.submit_misbehaviour(&mock_ctx, client_id, any_message, misbehaviour);

        assert!(result.is_ok(), "submit_misbehaviour failed: {:?}", result);

        // Verify the returned client state is frozen
        let misbehaviour_data = result.unwrap();
        let new_client_state = crate::client_state::ClientState::<SYNC_COMMITTEE_SIZE>::try_from(
            misbehaviour_data.new_any_client_state,
        )
        .unwrap();
        assert!(new_client_state.is_frozen());
        assert_eq!(new_client_state.frozen_height, Some(trusted_height));
    }

    #[test]
    fn test_update_client_with_misbehaviour() {
        use crate::misbehaviour::ETHEREUM_FINALIZED_HEADER_MISBEHAVIOUR_TYPE_URL;
        use ethereum_elc_proto::ibc::lightclients::ethereum::v1::FinalizedHeaderMisbehaviour as RawFinalizedHeaderMisbehaviour;
        use ethereum_light_client_types::consensus::convert_consensus_update_to_proto;
        use prost::Message;

        let fixture = TestFixture::new();

        let (update_1, _) = fixture.gen_update([1u8; 32].into(), 100);
        let (update_2, _) = fixture.gen_update([2u8; 32].into(), 100);

        let update_info_1 = to_consensus_update_info(update_1);
        let update_info_2 = to_consensus_update_info(update_2);
        let finalized_slot = update_info_1.finalized_beacon_header().slot;
        let timestamp_secs = compute_timestamp_at_slot(&fixture.ctx, finalized_slot).0;

        let consensus_state = fixture.consensus_state(
            fixture.period_1 + 1,
            [1u8; 32].into(),
            new_timestamp(timestamp_secs - 1000).unwrap(),
        );

        let trusted_height = Height::new(0, 1);
        let client_id = ClientId::new(ETHEREUM_CLIENT_TYPE, 0).unwrap();

        // Build raw misbehaviour proto
        let trusted_sync_committee = fixture.trusted_from_current(trusted_height, false);
        let raw_misbehaviour = RawFinalizedHeaderMisbehaviour {
            client_id: client_id.as_str().to_string(),
            trusted_sync_committee: Some(trusted_sync_committee.into()),
            consensus_update_1: Some(convert_consensus_update_to_proto(update_info_1).unwrap()),
            consensus_update_2: Some(convert_consensus_update_to_proto(update_info_2).unwrap()),
        };

        let mut buf = Vec::new();
        raw_misbehaviour.encode(&mut buf).unwrap();
        let any_message = Any::new(
            ETHEREUM_FINALIZED_HEADER_MISBEHAVIOUR_TYPE_URL.to_string(),
            buf,
        );

        let client_state = create_test_client_state_from_ctx(&fixture.ctx);
        let any_client_state: Any = client_state.try_into().unwrap();
        let any_consensus_state: Any = consensus_state.try_into().unwrap();

        let now = new_timestamp(timestamp_secs + 100).unwrap();
        let mut mock_ctx = MockHostContext::new(now);
        mock_ctx.setup_client(
            &client_id,
            any_client_state,
            any_consensus_state,
            trusted_height,
        );

        let client = EthereumLightClient::<SYNC_COMMITTEE_SIZE>;
        let result = client.update_client(&mock_ctx, client_id, any_message);

        assert!(
            result.is_ok(),
            "update_client with misbehaviour failed: {:?}",
            result
        );

        // Verify the result contains frozen client state
        match result.unwrap() {
            light_client::UpdateClientResult::Misbehaviour(misbehaviour_data) => {
                let new_client_state =
                    crate::client_state::ClientState::<SYNC_COMMITTEE_SIZE>::try_from(
                        misbehaviour_data.new_any_client_state,
                    )
                    .unwrap();
                assert!(new_client_state.is_frozen());
            }
            _ => panic!("Expected Misbehaviour result"),
        }
    }
}
