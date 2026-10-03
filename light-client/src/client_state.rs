use crate::consensus_state::ConsensusState;
use crate::errors::Error;
use crate::header::Header;
use crate::misbehaviour::Misbehaviour;
use crate::misc::to_lcp_height;
use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::time::Duration;
use ethereum_consensus::beacon::{Epoch, Root, Slot, Version};
use ethereum_consensus::fork::{ForkParameters, ForkSpec, BELLATRIX_INDEX};
use ethereum_consensus::types::{Address, H256, U64};
use ethereum_elc_proto::ibc::lightclients::ethereum::v1::ClientState as RawClientState;
use ethereum_light_client_proto::google::protobuf::Any as IBCAny;
use ethereum_light_client_proto::ibc::lightclients::ethereum::v1::{
    Fork as RawFork, ForkSpec as RawForkSpec,
};
use ethereum_light_client_types::client_state::ClientState as EthClientState;
use ethereum_light_client_types::commitment::verify_account_storage;
use ethereum_light_client_types::consensus::convert_proto_to_fork_parameters;
use ethereum_light_client_types::height::Height as LcTypesHeight;
use ethereum_light_client_types::time::{
    validate_header_timestamp_not_future, validate_state_timestamp_within_trusting_period,
};
use ethereum_light_client_types::update::compute_sync_committees;
use ethereum_light_client_types::update::TrustedConsensusState;
use ethereum_light_client_verifier::consensus::SyncProtocolVerifier;
use ethereum_light_client_verifier::context::{
    ChainConsensusVerificationContext, Fraction, LightClientContext,
};
use ethereum_light_client_verifier::execution::ExecutionVerifier;
use ethereum_light_client_verifier::updates::ConsensusUpdate;
use light_client::types::{Any, ClientId, Height, Time};
use prost::Message;
use serde::{Deserialize, Serialize};

/// The revision number for the Ethereum light client is always 0.
///
/// Therefore, in ethereum, the revision number is not used to determine the hard fork.
/// The current fork is determined by the client state's fork parameters.
pub const ETHEREUM_CLIENT_REVISION_NUMBER: u64 = 0;
pub const ETHEREUM_CLIENT_STATE_TYPE_URL: &str = "/ibc.lightclients.ethereum.v1.ClientState";
pub const ETHEREUM_ACCOUNT_STORAGE_ROOT_INDEX: usize = 2;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientState<const SYNC_COMMITTEE_SIZE: usize> {
    // Verification parameters
    /// `genesis_validators_root` of the target beacon chain's BeaconState
    pub genesis_validators_root: Root,
    /// https://github.com/ethereum/consensus-specs/blob/a09d0c321550c5411557674a981e2b444a1178c0/specs/altair/light-client/sync-protocol.md#misc
    pub min_sync_committee_participants: U64,
    /// `genesis_time` of the target beacon chain's BeaconState
    pub genesis_time: U64,
    /// fork parameters of the target beacon chain
    pub fork_parameters: ForkParameters,
    /// https://github.com/ethereum/consensus-specs/blob/a09d0c321550c5411557674a981e2b444a1178c0/configs/mainnet.yaml#L69
    pub seconds_per_slot: U64,
    /// https://github.com/ethereum/consensus-specs/blob/a09d0c321550c5411557674a981e2b444a1178c0/presets/mainnet/phase0.yaml#L36
    pub slots_per_epoch: Slot,
    /// https://github.com/ethereum/consensus-specs/blob/a09d0c321550c5411557674a981e2b444a1178c0/presets/mainnet/altair.yaml#L18
    pub epochs_per_sync_committee_period: Epoch,

    /// An address of IBC contract on execution layer
    pub ibc_address: Address,
    /// The IBC contract's base storage location for storing commitments
    /// https://github.com/hyperledger-labs/yui-ibc-solidity/blob/0e83dc7aadf71380dae6e346492e148685510663/docs/architecture.md#L46
    pub ibc_commitments_slot: H256,

    /// `trust_level` is threshold of sync committee participants to consider the attestation as valid. Highly recommended to be 2/3.
    pub trust_level: Fraction,
    /// `trusting_period` is the period in which the consensus state is considered trusted
    pub trusting_period: Duration,
    /// `max_clock_drift` defines how much new finalized header's time can drift into the future
    pub max_clock_drift: Duration,

    // State
    /// The latest block number of the stored consensus state
    pub latest_execution_block_number: U64,
    /// `frozen_height` is the height at which the client is considered frozen. If `None`, the client is unfrozen.
    pub frozen_height: Option<Height>,

    // Verifiers
    #[serde(skip)]
    pub consensus_verifier: SyncProtocolVerifier<
        SYNC_COMMITTEE_SIZE,
        TrustedConsensusState<SYNC_COMMITTEE_SIZE, ConsensusState>,
    >,
    #[serde(skip)]
    pub execution_verifier: ExecutionVerifier,
}

impl<const SYNC_COMMITTEE_SIZE: usize> EthClientState for ClientState<SYNC_COMMITTEE_SIZE> {
    fn latest_height(&self) -> LcTypesHeight {
        LcTypesHeight::new(
            ETHEREUM_CLIENT_REVISION_NUMBER,
            self.latest_execution_block_number.into(),
        )
    }

    fn ibc_commitments_slot(&self) -> H256 {
        self.ibc_commitments_slot
    }

    fn canonicalize(self) -> Self {
        let mut client_state = self;
        client_state.latest_execution_block_number = 0u64.into();
        client_state.frozen_height = None;
        client_state
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> ClientState<SYNC_COMMITTEE_SIZE> {
    /// Returns whether this client has been frozen due to misbehaviour.
    pub fn is_frozen(&self) -> bool {
        self.frozen_height.is_some()
    }

    pub fn with_frozen_height(self, h: Height) -> Self {
        Self {
            frozen_height: Some(h),
            ..self
        }
    }

    pub fn build_context(&self, current_timestamp: Time) -> impl ChainConsensusVerificationContext {
        LightClientContext::new(
            self.fork_parameters.clone(),
            self.seconds_per_slot,
            self.slots_per_epoch,
            self.epochs_per_sync_committee_period,
            self.genesis_time,
            self.genesis_validators_root,
            self.min_sync_committee_participants.0 as usize,
            self.trust_level.clone(),
            current_timestamp.as_unix_timestamp_secs().into(),
        )
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.genesis_validators_root == Root::default() {
            Err(Error::UninitializedClientStateField(
                "genesis_validators_root",
            ))
        } else if self.min_sync_committee_participants == U64::default() {
            Err(Error::UninitializedClientStateField(
                "min_sync_committee_participants",
            ))
        } else if self.genesis_time == U64::default() {
            Err(Error::UninitializedClientStateField("genesis_time"))
        } else if self.fork_parameters == ForkParameters::default() {
            Err(Error::UninitializedClientStateField("fork_parameters"))
        } else if self.fork_parameters.forks().len() <= BELLATRIX_INDEX {
            Err(Error::MissingBellatrixFork)
        } else if self.seconds_per_slot == U64::default() {
            Err(Error::UninitializedClientStateField("seconds_per_slot"))
        } else if self.slots_per_epoch == Slot::default() {
            Err(Error::UninitializedClientStateField("slots_per_epoch"))
        } else if self.epochs_per_sync_committee_period == U64::default() {
            Err(Error::UninitializedClientStateField(
                "epochs_per_sync_committee_period",
            ))
        } else if self.ibc_address == Address::default() {
            Err(Error::UninitializedClientStateField("ibc_address"))
        } else if self.trust_level == Fraction::default() {
            Err(Error::UninitializedClientStateField("trust_level"))
        } else if self.trusting_period == Duration::default() {
            Err(Error::UninitializedClientStateField("trusting_period"))
        } else if self.latest_execution_block_number == U64::default() {
            Err(Error::UninitializedClientStateField(
                "latest_execution_block_number",
            ))
        } else {
            Ok(())
        }
    }

    pub fn check_header_and_update_state(
        &self,
        now: Time,
        consensus_state: &ConsensusState,
        header: Header<SYNC_COMMITTEE_SIZE>,
    ) -> Result<(ClientState<SYNC_COMMITTEE_SIZE>, ConsensusState), Error> {
        let cc = self.build_context(now);
        header.validate()?;

        let trusted_sync_committee = header.trusted_sync_committee;
        let trusted_consensus_state = TrustedConsensusState::new(
            consensus_state.clone(),
            trusted_sync_committee.sync_committee,
            trusted_sync_committee.is_next,
        )?;
        let consensus_update = header.consensus_update;
        let execution_update = header.execution_update;
        let account_update = header.account_update;

        self.consensus_verifier
            .validate_updates(
                &cc,
                &trusted_consensus_state,
                &consensus_update,
                &execution_update,
            )
            .map_err(Error::Verification)?;

        // Derived only now: post-Gloas this reads the RLP execution header, which
        // `validate_updates` has just bound to the consensus update.
        let header_timestamp =
            execution_update.timestamp(&cc, consensus_update.finalized_beacon_header().slot)?;
        let header_timestamp = Time::from_unix_timestamp_nanos(header_timestamp)?;

        verify_account_storage(
            &self.execution_verifier,
            execution_update.state_root,
            &self.ibc_address,
            &account_update,
        )?;

        // check if the current timestamp is within the trusting period
        validate_state_timestamp_within_trusting_period(
            now.as_unix_timestamp_nanos(),
            self.trusting_period,
            consensus_state.timestamp.as_unix_timestamp_nanos(),
        )?;
        // check if the header timestamp does not indicate a future time
        validate_header_timestamp_not_future(
            now.as_unix_timestamp_nanos(),
            self.max_clock_drift,
            header_timestamp.as_unix_timestamp_nanos(),
        )?;

        let finalized_slot = consensus_update.finalized_header.0.slot;
        let new_sync_committee = compute_sync_committees(&cc, consensus_state, consensus_update)?;

        // apply updates to state
        let mut new_client_state: ClientState<SYNC_COMMITTEE_SIZE> = self.clone();
        if new_client_state.latest_execution_block_number < execution_update.block_number {
            new_client_state.latest_execution_block_number = execution_update.block_number;
        }
        let mut new_consensus_state = consensus_state.clone();
        new_consensus_state.slot = finalized_slot;
        new_consensus_state.storage_root = account_update.account_storage_root;
        new_consensus_state.timestamp = header_timestamp;
        new_consensus_state.current_sync_committee = new_sync_committee.current_sync_committee;
        new_consensus_state.next_sync_committee = new_sync_committee.next_sync_committee;

        Ok((new_client_state, new_consensus_state))
    }

    pub fn check_misbehaviour_and_update_state(
        &self,
        now: Time,
        client_id: &ClientId,
        consensus_state: &ConsensusState,
        misbehaviour: Misbehaviour<SYNC_COMMITTEE_SIZE>,
    ) -> Result<ClientState<SYNC_COMMITTEE_SIZE>, Error> {
        misbehaviour.validate()?;
        if &misbehaviour.client_id != client_id {
            return Err(Error::UnexpectedClientIdInMisbehaviour {
                expected: client_id.clone(),
                actual: misbehaviour.client_id,
            });
        }

        let cc = self.build_context(now);
        let trusted_consensus_state = TrustedConsensusState::new(
            consensus_state.clone(),
            misbehaviour.trusted_sync_committee.sync_committee,
            misbehaviour.trusted_sync_committee.is_next,
        )?;

        self.consensus_verifier
            .validate_misbehaviour(&cc, &trusted_consensus_state, misbehaviour.data)
            .map_err(Error::Verification)?;

        validate_state_timestamp_within_trusting_period(
            now.as_unix_timestamp_nanos(),
            self.trusting_period,
            consensus_state.timestamp.as_unix_timestamp_nanos(),
        )?;

        // found misbehaviour
        Ok(self
            .clone()
            .with_frozen_height(to_lcp_height(misbehaviour.trusted_sync_committee.height)))
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> TryFrom<RawClientState>
    for ClientState<SYNC_COMMITTEE_SIZE>
{
    type Error = Error;

    fn try_from(value: RawClientState) -> Result<Self, Self::Error> {
        let raw_fork_parameters = value
            .fork_parameters
            .ok_or(Error::proto_missing("fork_parameters"))?;
        let fork_parameters = convert_proto_to_fork_parameters(raw_fork_parameters)?;
        let trust_level = value
            .trust_level
            .ok_or(Error::proto_missing("trust_level"))?;
        let frozen_height = value
            .frozen_height
            .map(|h| Height::new(h.revision_number, h.revision_height));
        if value.genesis_validators_root.len() != 32 {
            return Err(Error::InvalidRawClientState {
                reason: format!(
                    "invalid genesis_validators_root length: {}",
                    value.genesis_validators_root.len()
                ),
            });
        }
        if value.ibc_commitments_slot.len() != 32 {
            return Err(Error::InvalidRawClientState {
                reason: format!(
                    "invalid ibc_commitments_slot length: {}",
                    value.ibc_commitments_slot.len()
                ),
            });
        }
        Ok(Self {
            genesis_validators_root: H256::from_slice(&value.genesis_validators_root),
            min_sync_committee_participants: value.min_sync_committee_participants.into(),
            genesis_time: value.genesis_time.into(),
            fork_parameters,
            seconds_per_slot: value.seconds_per_slot.into(),
            slots_per_epoch: value.slots_per_epoch.into(),
            epochs_per_sync_committee_period: value.epochs_per_sync_committee_period.into(),
            ibc_address: value
                .ibc_address
                .as_slice()
                .try_into()
                .map_err(|e| Error::UnexpectedStoreAddress(format!("{:?}", e)))?,
            ibc_commitments_slot: H256::from_slice(&value.ibc_commitments_slot),
            trust_level: Fraction::new(trust_level.numerator, trust_level.denominator)
                .map_err(Error::Verification)?,
            trusting_period: value
                .trusting_period
                .ok_or(Error::MissingTrustingPeriod)?
                .try_into()
                .map_err(|_| Error::MissingTrustingPeriod)?,
            max_clock_drift: value
                .max_clock_drift
                .ok_or(Error::NegativeMaxClockDrift)?
                .try_into()
                .map_err(|_| Error::NegativeMaxClockDrift)?,
            latest_execution_block_number: value.latest_execution_block_number.into(),
            frozen_height,
            consensus_verifier: Default::default(),
            execution_verifier: Default::default(),
        })
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> From<ClientState<SYNC_COMMITTEE_SIZE>> for RawClientState {
    fn from(value: ClientState<SYNC_COMMITTEE_SIZE>) -> Self {
        use ethereum_light_client_proto::ibc::core::client::v1::Height as ProtoHeight;
        use ethereum_light_client_proto::ibc::lightclients::ethereum::v1::{
            ForkParameters as ProtoForkParameters, Fraction as ProtoFraction,
        };

        fn make_fork(version: &Version, epoch: U64, spec: ForkSpec) -> RawFork {
            RawFork {
                version: version_to_bytes(version),
                epoch: epoch.into(),
                spec: Some(RawForkSpec {
                    finalized_root_gindex: spec.finalized_root_gindex,
                    current_sync_committee_gindex: spec.current_sync_committee_gindex,
                    next_sync_committee_gindex: spec.next_sync_committee_gindex,
                    execution_payload_gindex: spec.execution_payload_gindex,
                    execution_payload_state_root_gindex: spec.execution_payload_state_root_gindex,
                    execution_payload_block_number_gindex: spec
                        .execution_payload_block_number_gindex,
                    execution_block_hash_gindex: spec.execution_block_hash_gindex,
                }),
            }
        }

        fn version_to_bytes(version: &Version) -> Vec<u8> {
            version.0.to_vec()
        }

        let fork_parameters = value.fork_parameters;

        Self {
            genesis_validators_root: value.genesis_validators_root.as_bytes().to_vec(),
            min_sync_committee_participants: value.min_sync_committee_participants.into(),
            genesis_time: value.genesis_time.into(),
            fork_parameters: Some(ProtoForkParameters {
                genesis_fork_version: version_to_bytes(fork_parameters.genesis_version()),
                forks: fork_parameters
                    .forks()
                    .iter()
                    .map(|f| make_fork(&f.version, f.epoch, f.spec.clone()))
                    .collect(),
            }),
            seconds_per_slot: value.seconds_per_slot.into(),
            slots_per_epoch: value.slots_per_epoch.into(),
            epochs_per_sync_committee_period: value.epochs_per_sync_committee_period.into(),
            ibc_address: value.ibc_address.0.to_vec(),
            ibc_commitments_slot: value.ibc_commitments_slot.as_bytes().to_vec(),
            trust_level: Some(ProtoFraction {
                numerator: value.trust_level.numerator(),
                denominator: value.trust_level.denominator(),
            }),
            trusting_period: Some(value.trusting_period.into()),
            max_clock_drift: Some(value.max_clock_drift.into()),
            latest_execution_block_number: value.latest_execution_block_number.into(),
            frozen_height: value.frozen_height.map(|h| ProtoHeight {
                revision_number: h.revision_number(),
                revision_height: h.revision_height(),
            }),
        }
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> TryFrom<IBCAny> for ClientState<SYNC_COMMITTEE_SIZE> {
    type Error = Error;

    fn try_from(any: IBCAny) -> Result<Self, Self::Error> {
        if any.type_url != ETHEREUM_CLIENT_STATE_TYPE_URL {
            return Err(Error::UnknownClientStateType(any.type_url));
        }
        RawClientState::decode(any.value.as_slice())
            .map_err(Error::ProtoDecode)?
            .try_into()
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> TryFrom<ClientState<SYNC_COMMITTEE_SIZE>> for IBCAny {
    type Error = Error;

    fn try_from(value: ClientState<SYNC_COMMITTEE_SIZE>) -> Result<Self, Self::Error> {
        let value: RawClientState = value.into();
        let mut v = Vec::new();
        value.encode(&mut v).map_err(Error::ProtoEncode)?;
        Ok(Self {
            type_url: ETHEREUM_CLIENT_STATE_TYPE_URL.to_string(),
            value: v,
        })
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> TryFrom<ClientState<SYNC_COMMITTEE_SIZE>> for Any {
    type Error = Error;
    fn try_from(value: ClientState<SYNC_COMMITTEE_SIZE>) -> Result<Self, Error> {
        Ok(IBCAny::try_from(value)?.into())
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> TryFrom<Any> for ClientState<SYNC_COMMITTEE_SIZE> {
    type Error = Error;

    fn try_from(any: Any) -> Result<Self, Self::Error> {
        IBCAny::from(any).try_into()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ethereum_consensus::fork::{
        altair::ALTAIR_FORK_SPEC, bellatrix::BELLATRIX_FORK_SPEC, capella::CAPELLA_FORK_SPEC,
        deneb::DENEB_FORK_SPEC, ForkParameter,
    };
    use ethereum_consensus::preset::minimal::PRESET;
    use ethereum_light_client_verifier::context::Fraction;
    use hex_literal::hex;

    pub type TestClientState = ClientState<{ PRESET.SYNC_COMMITTEE_SIZE }>;

    /// Creates a valid test client state for testing purposes.
    pub fn create_test_client_state() -> TestClientState {
        TestClientState {
            genesis_validators_root: H256::from_slice(&[1u8; 32]),
            min_sync_committee_participants: 1u64.into(),
            genesis_time: 1u64.into(),
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
            ibc_address: Address(hex!("ff77D90D6aA12db33d3Ba50A34fB25401f6e4c4F")),
            ibc_commitments_slot: H256::from_slice(&[2u8; 32]),
            trust_level: Fraction::new(2, 3).unwrap(),
            trusting_period: Duration::from_secs(60 * 60 * 27),
            max_clock_drift: Duration::from_secs(60),
            latest_execution_block_number: 1u64.into(),
            frozen_height: None,
            consensus_verifier: Default::default(),
            execution_verifier: Default::default(),
        }
    }

    #[test]
    fn test_client_state_default() {
        let state = TestClientState::default();
        assert_eq!(state.genesis_validators_root, Root::default());
        assert_eq!(state.min_sync_committee_participants, U64::default());
        assert_eq!(state.genesis_time, U64::default());
        assert_eq!(state.frozen_height, None);
        assert!(!state.is_frozen());
    }

    #[test]
    fn test_client_state_validate_default_fails() {
        let state = TestClientState::default();
        let result = state.validate();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            Error::UninitializedClientStateField("genesis_validators_root")
        ));
    }

    #[test]
    fn test_client_state_validate_success() {
        let state = create_test_client_state();
        let result = state.validate();
        assert!(result.is_ok(), "{:?}", result);
    }

    #[test]
    fn test_client_state_is_frozen() {
        let state = TestClientState::default();
        assert!(!state.is_frozen());

        let frozen_state = state.with_frozen_height(Height::new(0, 100));
        assert!(frozen_state.is_frozen());
        assert_eq!(frozen_state.frozen_height, Some(Height::new(0, 100)));
    }

    #[test]
    fn test_client_state_latest_height() {
        let state = TestClientState {
            latest_execution_block_number: U64(12345),
            ..Default::default()
        };

        let height = state.latest_height();
        assert_eq!(height.revision_number(), ETHEREUM_CLIENT_REVISION_NUMBER);
        assert_eq!(height.revision_height(), 12345);
    }

    #[test]
    fn test_client_state_canonicalize() {
        let state = TestClientState {
            latest_execution_block_number: U64(12345),
            frozen_height: Some(Height::new(0, 100)),
            ..Default::default()
        };

        let canonicalized = state.canonicalize();
        assert_eq!(canonicalized.latest_execution_block_number, U64(0));
        assert_eq!(canonicalized.frozen_height, None);
    }

    #[test]
    fn test_client_state_ibc_commitments_slot() {
        let mut state = TestClientState::default();
        let slot = H256::from_slice(&[0xab; 32]);
        state.ibc_commitments_slot = slot;

        assert_eq!(state.ibc_commitments_slot(), slot);
    }

    #[test]
    fn test_ethereum_client_revision_number() {
        assert_eq!(ETHEREUM_CLIENT_REVISION_NUMBER, 0);
    }

    #[test]
    fn test_client_state_validate_missing_min_sync_committee_participants() {
        let state = TestClientState {
            genesis_validators_root: H256::from_slice(&[1u8; 32]),
            min_sync_committee_participants: U64::default(),
            ..Default::default()
        };
        let result = state.validate();
        assert!(matches!(
            result.unwrap_err(),
            Error::UninitializedClientStateField("min_sync_committee_participants")
        ));
    }

    #[test]
    fn test_client_state_validate_missing_genesis_time() {
        let state = TestClientState {
            genesis_validators_root: H256::from_slice(&[1u8; 32]),
            min_sync_committee_participants: U64(1),
            genesis_time: U64::default(),
            ..Default::default()
        };
        let result = state.validate();
        assert!(matches!(
            result.unwrap_err(),
            Error::UninitializedClientStateField("genesis_time")
        ));
    }

    #[test]
    fn test_client_state_validate_missing_bellatrix_fork() {
        let state = TestClientState {
            genesis_validators_root: H256::from_slice(&[1u8; 32]),
            min_sync_committee_participants: 1u64.into(),
            genesis_time: 1u64.into(),
            fork_parameters: ForkParameters::new(
                Version([0, 0, 0, 1]),
                vec![ForkParameter::new(
                    Version([1, 0, 0, 1]),
                    U64(0),
                    ALTAIR_FORK_SPEC,
                )],
            )
            .unwrap(),
            seconds_per_slot: PRESET.SECONDS_PER_SLOT,
            slots_per_epoch: PRESET.SLOTS_PER_EPOCH,
            epochs_per_sync_committee_period: PRESET.EPOCHS_PER_SYNC_COMMITTEE_PERIOD,
            ..Default::default()
        };
        let result = state.validate();
        assert!(matches!(result.unwrap_err(), Error::MissingBellatrixFork));
    }

    #[test]
    fn test_client_state_proto_conversion() {
        let client_state = create_test_client_state();

        // Convert to Any and back
        let any: Any = client_state.clone().try_into().unwrap();
        let client_state2 = TestClientState::try_from(any).unwrap();

        assert_eq!(client_state, client_state2);
    }

    #[test]
    fn test_client_state_proto_conversion_ibc_any() {
        let client_state = create_test_client_state();

        // Convert to IBCAny and back
        let ibc_any: IBCAny = client_state.clone().try_into().unwrap();
        assert_eq!(ibc_any.type_url, ETHEREUM_CLIENT_STATE_TYPE_URL);

        let client_state2 = TestClientState::try_from(ibc_any).unwrap();
        assert_eq!(client_state, client_state2);
    }

    #[test]
    fn test_client_state_from_any_unknown_type() {
        let any = IBCAny {
            type_url: "/unknown.type".to_string(),
            value: vec![],
        };
        let result = TestClientState::try_from(any);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            Error::UnknownClientStateType(ref url) if url == "/unknown.type"
        ));
    }

    #[test]
    fn test_client_state_build_context() {
        let client_state = create_test_client_state();
        let now = Time::from_unix_timestamp_nanos(1_000_000_000_000_000_000).unwrap();
        let _ctx = client_state.build_context(now);
        // If it doesn't panic, the context was built successfully
    }

    /// Regression: malformed fixed-length fields must return `InvalidRawClientState`
    /// instead of panicking in `H256::from_slice` / `copy_from_slice`. A panic inside
    /// the enclave is `sgx_abort` (SIGILL) = crash/DoS, so each length guard is tested.
    #[test]
    fn test_client_state_try_from_invalid_genesis_validators_root_length() {
        for bad_len in [0usize, 31, 33, 64] {
            let mut raw: RawClientState = create_test_client_state().into();
            raw.genesis_validators_root = vec![1u8; bad_len];
            let result = TestClientState::try_from(raw);
            assert!(
                matches!(result, Err(Error::InvalidRawClientState { .. })),
                "genesis_validators_root len={} must be rejected with InvalidRawClientState",
                bad_len
            );
        }
    }

    #[test]
    fn test_client_state_try_from_invalid_ibc_commitments_slot_length() {
        for bad_len in [0usize, 31, 33, 64] {
            let mut raw: RawClientState = create_test_client_state().into();
            raw.ibc_commitments_slot = vec![1u8; bad_len];
            let result = TestClientState::try_from(raw);
            assert!(
                matches!(result, Err(Error::InvalidRawClientState { .. })),
                "ibc_commitments_slot len={} must be rejected with InvalidRawClientState",
                bad_len
            );
        }
    }

    #[test]
    fn test_client_state_with_frozen_height() {
        let state = create_test_client_state();
        assert!(!state.is_frozen());

        let frozen = state.with_frozen_height(Height::new(0, 100));
        assert!(frozen.is_frozen());
        assert_eq!(frozen.frozen_height, Some(Height::new(0, 100)));
    }
}

/// Integration tests for check_header_and_update_state and check_misbehaviour_and_update_state
#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::consensus_state::ConsensusState;
    use crate::header::Header;
    use crate::misbehaviour::Misbehaviour;
    use crate::misc::new_timestamp;
    use crate::test_utils::{
        account_proof, create_test_client_state_from_ctx, gloas_devnet, to_consensus_update_info,
        TestClientState, TestFixture,
    };
    use core::str::FromStr;
    use ethereum_consensus::compute::compute_timestamp_at_slot;
    use ethereum_light_client_types::consensus::{AccountUpdateInfo, ExecutionUpdateInfo};
    use ethereum_light_client_verifier::misbehaviour::{
        FinalizedHeaderMisbehaviour, Misbehaviour as MisbehaviourData,
    };
    use ethereum_light_client_verifier::updates::ConsensusUpdate;

    #[test]
    fn test_check_header_with_valid_account_proof() {
        let client_state = TestClientState {
            ibc_address: account_proof::get_address(),
            execution_verifier: Default::default(),
            ..Default::default()
        };

        let account_update = AccountUpdateInfo {
            account_proof: account_proof::get_proof(),
            account_storage_root: account_proof::get_storage_root(),
        };

        // Verify account storage works with valid proofs
        let result = verify_account_storage(
            &client_state.execution_verifier,
            account_proof::get_state_root(),
            &client_state.ibc_address,
            &account_update,
        );

        assert!(
            result.is_ok(),
            "Account storage verification failed: {:?}",
            result
        );
    }

    #[test]
    fn test_check_header_with_invalid_account_proof() {
        let client_state = TestClientState {
            ibc_address: account_proof::get_address(),
            execution_verifier: Default::default(),
            ..Default::default()
        };

        // Invalid proof (random bytes)
        let account_update = AccountUpdateInfo {
            account_proof: vec![vec![1, 2, 3]],
            account_storage_root: account_proof::get_storage_root(),
        };

        let result = verify_account_storage(
            &client_state.execution_verifier,
            account_proof::get_state_root(),
            &client_state.ibc_address,
            &account_update,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_check_header_with_mismatched_storage_root() {
        let client_state = TestClientState {
            ibc_address: account_proof::get_address(),
            execution_verifier: Default::default(),
            ..Default::default()
        };

        // Wrong storage root
        let account_update = AccountUpdateInfo {
            account_proof: account_proof::get_proof(),
            account_storage_root: H256::from_slice(&[0xaa; 32]),
        };

        let result = verify_account_storage(
            &client_state.execution_verifier,
            account_proof::get_state_root(),
            &client_state.ibc_address,
            &account_update,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_check_header_and_update_state_success() {
        let fixture = TestFixture::new();

        // Use the real state_root from the test account proof
        let execution_state_root = account_proof::get_state_root();
        let dummy_execution_block_number = 100u64;

        let (update, execution_update) =
            fixture.gen_update(execution_state_root, dummy_execution_block_number);
        let update_info = to_consensus_update_info(update);
        let execution_update_info = ExecutionUpdateInfo {
            state_root: execution_update.state_root,
            state_root_branch: execution_update.state_root_branch,
            block_number: execution_update.block_number,
            block_number_branch: execution_update.block_number_branch,
            rlp: vec![],
            block_hash: H256::default(),
            block_hash_branch: vec![],
        };
        let finalized_slot = update_info.finalized_beacon_header().slot;
        let timestamp_secs = compute_timestamp_at_slot(&fixture.ctx, finalized_slot).0;

        // Create consensus state with slot in period 1 (same as signature period)
        let consensus_state_slot = fixture.period_1 + 1;
        let consensus_state = fixture.consensus_state(
            consensus_state_slot,
            account_proof::get_storage_root(),
            new_timestamp(timestamp_secs - 1000).unwrap(),
        );

        // Create header with matching account proof
        // Note: is_next: false because the signature was created by current_sync_committee
        let header = Header {
            trusted_sync_committee: fixture.trusted_from_current(Height::new(0, 1), false),
            consensus_update: update_info.clone(),
            execution_update: execution_update_info,
            account_update: AccountUpdateInfo {
                account_proof: account_proof::get_proof(),
                account_storage_root: account_proof::get_storage_root(),
            },
        };

        // Create client state with the correct IBC address for the account proof
        let mut client_state = create_test_client_state_from_ctx(&fixture.ctx);
        client_state.ibc_address = account_proof::get_address();

        let now = new_timestamp(timestamp_secs + 100).unwrap();
        let result = client_state.check_header_and_update_state(now, &consensus_state, header);

        // This should succeed!
        assert!(
            result.is_ok(),
            "check_header_and_update_state failed: {:?}",
            result
        );

        let (new_client_state, new_consensus_state) = result.unwrap();

        // Verify the state was updated correctly
        assert_eq!(
            new_client_state.latest_execution_block_number,
            dummy_execution_block_number.into()
        );
        // The storage root in consensus_state is the IBC contract's account storage root
        assert_eq!(
            new_consensus_state.storage_root,
            account_proof::get_storage_root()
        );
        // Regression: `storage_root` must be the account storage root, NOT the execution
        // state root (the two differ; using the state root breaks `verify_membership`).
        assert_ne!(new_consensus_state.storage_root, execution_state_root);
        // Regression: the consensus state slot must advance to the update's finalized
        // slot. A frozen slot freezes `current_period` and breaks sync-committee period
        // accounting across period boundaries.
        assert_eq!(new_consensus_state.slot, finalized_slot);
        assert_ne!(new_consensus_state.slot, consensus_state_slot);
    }

    /// Regression test for cross-period updates (#1).
    ///
    /// Starts from a trusted consensus state in period 0 and applies an update whose
    /// finalized header is in period 1 (`is_next = true`). The produced consensus state
    /// must advance to period 1. If the slot is not updated, `current_period` stays
    /// frozen at 0 and a subsequent update fails with `UnexpectedSignaturePeriod`.
    #[test]
    fn test_check_header_and_update_state_period_crossing() {
        let fixture = TestFixture::new();
        let execution_state_root = account_proof::get_state_root();

        let (update, execution_update) = fixture.gen_update(execution_state_root, 100);
        let update_info = to_consensus_update_info(update);
        let execution_update_info = ExecutionUpdateInfo {
            state_root: execution_update.state_root,
            state_root_branch: execution_update.state_root_branch,
            block_number: execution_update.block_number,
            block_number_branch: execution_update.block_number_branch,
            rlp: vec![],
            block_hash: H256::default(),
            block_hash_branch: vec![],
        };
        let finalized_slot = update_info.finalized_beacon_header().slot;
        let timestamp_secs = compute_timestamp_at_slot(&fixture.ctx, finalized_slot).0;

        // Trusted state is in PERIOD 0 (slot 32 < 64); the update's finalized header is
        // in period 1, so the update crosses a sync committee period boundary.
        let store_slot: U64 = 32u64.into();
        let consensus_state = ConsensusState {
            slot: store_slot,
            storage_root: account_proof::get_storage_root(),
            timestamp: new_timestamp(timestamp_secs - 1000).unwrap(),
            // `current` is unused when is_next = true; `next` must match the header committee
            // (the period-1 committee that signed the update).
            current_sync_committee: fixture
                .current_sync_committee()
                .to_committee()
                .aggregate_pubkey
                .clone(),
            next_sync_committee: fixture
                .current_sync_committee()
                .to_committee()
                .aggregate_pubkey
                .clone(),
        };

        let header = Header {
            trusted_sync_committee: fixture.trusted_from_current(Height::new(0, 1), true),
            consensus_update: update_info,
            execution_update: execution_update_info,
            account_update: AccountUpdateInfo {
                account_proof: account_proof::get_proof(),
                account_storage_root: account_proof::get_storage_root(),
            },
        };

        let mut client_state = create_test_client_state_from_ctx(&fixture.ctx);
        client_state.ibc_address = account_proof::get_address();

        let now = new_timestamp(timestamp_secs + 100).unwrap();
        let result = client_state.check_header_and_update_state(now, &consensus_state, header);
        assert!(result.is_ok(), "cross-period update failed: {:?}", result);

        let (_, new_consensus_state) = result.unwrap();
        // The store period must advance from 0 to 1 (slot updated to the finalized slot).
        assert_eq!(new_consensus_state.slot, finalized_slot);
        assert_eq!(
            new_consensus_state.current_period(&fixture.ctx),
            1u64.into()
        );
    }

    // ========================================================================
    // Tests for check_misbehaviour_and_update_state
    // ========================================================================

    #[test]
    fn test_check_misbehaviour_client_id_mismatch() {
        let fixture = TestFixture::new();
        let dummy_execution_state_root: H256 = [1u8; 32].into();

        let (update_1, _) = fixture.gen_update(dummy_execution_state_root, 100);
        let (update_2, _) = fixture.gen_update([2u8; 32].into(), 100);

        let update_info_1 = to_consensus_update_info(update_1);
        let update_info_2 = to_consensus_update_info(update_2);
        let finalized_slot = update_info_1.finalized_beacon_header().slot;
        let timestamp_secs = compute_timestamp_at_slot(&fixture.ctx, finalized_slot).0;

        let consensus_state = fixture.consensus_state(
            fixture.period_1 + 1,
            dummy_execution_state_root,
            new_timestamp(timestamp_secs - 1000).unwrap(),
        );

        // Misbehaviour with different client_id
        let misbehaviour = Misbehaviour {
            client_id: ClientId::from_str("ethereum-999").unwrap(), // Different client_id
            trusted_sync_committee: fixture.trusted_from_current(Height::new(0, 1), false),
            data: MisbehaviourData::FinalizedHeader(FinalizedHeaderMisbehaviour {
                consensus_update_1: update_info_1,
                consensus_update_2: update_info_2,
            }),
        };

        let client_state = create_test_client_state_from_ctx(&fixture.ctx);
        let expected_client_id = ClientId::from_str("ethereum-0").unwrap();
        let now = new_timestamp(timestamp_secs + 100).unwrap();

        let result = client_state.check_misbehaviour_and_update_state(
            now,
            &expected_client_id,
            &consensus_state,
            misbehaviour,
        );

        assert!(result.is_err());
        match result.unwrap_err() {
            Error::UnexpectedClientIdInMisbehaviour { expected, actual } => {
                assert_eq!(expected.as_str(), "ethereum-0");
                assert_eq!(actual.as_str(), "ethereum-999");
            }
            e => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_check_misbehaviour_and_update_state_success() {
        let fixture = TestFixture::new();

        // Create two updates with different execution state roots at the same finalized slot
        // This constitutes a finalized header misbehaviour
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

        let trusted_height = Height::new(0, 50);
        let misbehaviour = Misbehaviour {
            client_id: ClientId::from_str("ethereum-0").unwrap(),
            trusted_sync_committee: fixture.trusted_from_current(trusted_height, false),
            data: MisbehaviourData::FinalizedHeader(FinalizedHeaderMisbehaviour {
                consensus_update_1: update_info_1,
                consensus_update_2: update_info_2,
            }),
        };

        let client_state = create_test_client_state_from_ctx(&fixture.ctx);
        assert!(!client_state.is_frozen());

        let client_id = ClientId::from_str("ethereum-0").unwrap();
        let now = new_timestamp(timestamp_secs + 100).unwrap();

        let result = client_state.check_misbehaviour_and_update_state(
            now,
            &client_id,
            &consensus_state,
            misbehaviour,
        );

        // Misbehaviour should be detected and client should be frozen
        assert!(
            result.is_ok(),
            "check_misbehaviour_and_update_state failed: {:?}",
            result
        );

        let frozen_client_state = result.unwrap();
        assert!(frozen_client_state.is_frozen());
        assert_eq!(frozen_client_state.frozen_height, Some(trusted_height));
    }

    #[test]
    fn test_check_misbehaviour_trusting_period_expired() {
        let fixture = TestFixture::new();

        let (update_1, _) = fixture.gen_update([1u8; 32].into(), 100);
        let (update_2, _) = fixture.gen_update([2u8; 32].into(), 100);

        let update_info_1 = to_consensus_update_info(update_1);
        let update_info_2 = to_consensus_update_info(update_2);
        let finalized_slot = update_info_1.finalized_beacon_header().slot;
        let timestamp_secs = compute_timestamp_at_slot(&fixture.ctx, finalized_slot).0;

        // Create consensus state with old timestamp (outside trusting period)
        let old_timestamp_secs = timestamp_secs - (60 * 60 * 24 * 8); // 8 days ago
        let consensus_state = fixture.consensus_state(
            fixture.period_1 + 1,
            [1u8; 32].into(),
            new_timestamp(old_timestamp_secs).unwrap(),
        );

        let misbehaviour = Misbehaviour {
            client_id: ClientId::from_str("ethereum-0").unwrap(),
            trusted_sync_committee: fixture.trusted_from_current(Height::new(0, 50), false),
            data: MisbehaviourData::FinalizedHeader(FinalizedHeaderMisbehaviour {
                consensus_update_1: update_info_1,
                consensus_update_2: update_info_2,
            }),
        };

        let client_state = create_test_client_state_from_ctx(&fixture.ctx);
        let client_id = ClientId::from_str("ethereum-0").unwrap();
        // Now is much later than consensus_state.timestamp
        let now = new_timestamp(timestamp_secs + 100).unwrap();

        let result = client_state.check_misbehaviour_and_update_state(
            now,
            &client_id,
            &consensus_state,
            misbehaviour,
        );

        // Should fail due to trusting period expiration
        assert!(result.is_err());
        // The error comes from validate_state_timestamp_within_trusting_period
        // which returns EthereumLightClientTypes error
        assert!(matches!(
            result.unwrap_err(),
            Error::EthereumLightClientTypes(_)
        ));
    }

    /// The trusted state is finalized in Fulu and every following update is finalized in
    /// Gloas: the period 1 update (Fulu -> Gloas, a period transition since the fork activates
    /// at a period boundary), the period 2 update (a period transition inside Gloas) and a
    /// finality update inside period 2.
    #[test]
    fn test_check_header_fulu_to_gloas() {
        let client_state = gloas_devnet::client_state();
        let consensus_state = gloas_devnet::consensus_state();

        // 1. the first update after the fork: finalized in Gloas, trusted state in Fulu
        let header = gloas_devnet::period_1_header();
        assert!(!header.execution_update.rlp.is_empty());
        let now = new_timestamp(gloas_devnet::PERIOD_1_NOW).unwrap();
        let (client_state, consensus_state) = client_state
            .check_header_and_update_state(now, &consensus_state, header)
            .unwrap();
        assert_eq!(
            consensus_state.slot,
            U64(gloas_devnet::PERIOD_1_FINALIZED_SLOT)
        );
        assert_eq!(
            consensus_state.timestamp,
            new_timestamp(gloas_devnet::PERIOD_1_HEADER_TIMESTAMP).unwrap()
        );
        assert_ne!(consensus_state.storage_root, H256::default());
        assert_eq!(
            client_state.latest_execution_block_number,
            U64(gloas_devnet::PERIOD_1_EXECUTION_BLOCK_NUMBER)
        );

        // 2. a period transition inside Gloas
        let header = gloas_devnet::period_2_header();
        let now = new_timestamp(gloas_devnet::PERIOD_2_NOW).unwrap();
        let (client_state, consensus_state) = client_state
            .check_header_and_update_state(now, &consensus_state, header)
            .unwrap();
        assert_eq!(
            consensus_state.slot,
            U64(gloas_devnet::PERIOD_2_FINALIZED_SLOT)
        );
        assert_eq!(
            consensus_state.timestamp,
            new_timestamp(gloas_devnet::PERIOD_2_HEADER_TIMESTAMP).unwrap()
        );
        assert_eq!(
            client_state.latest_execution_block_number,
            U64(gloas_devnet::PERIOD_2_EXECUTION_BLOCK_NUMBER)
        );

        // 3. a finality update inside the same period
        let header = gloas_devnet::steady_header();
        let now = new_timestamp(gloas_devnet::STEADY_NOW).unwrap();
        let (client_state, consensus_state) = client_state
            .check_header_and_update_state(now, &consensus_state, header)
            .unwrap();
        assert_eq!(
            consensus_state.slot,
            U64(gloas_devnet::STEADY_FINALIZED_SLOT)
        );
        assert_eq!(
            consensus_state.timestamp,
            new_timestamp(gloas_devnet::STEADY_HEADER_TIMESTAMP).unwrap()
        );
        assert_eq!(
            client_state.latest_execution_block_number,
            U64(gloas_devnet::STEADY_EXECUTION_BLOCK_NUMBER)
        );
    }
}
