//! Test utilities for Ethereum light client tests.
//!
//! This module provides common helper functions and types for testing,
//! reducing code duplication across test modules.

use ethereum_consensus::config;
use ethereum_consensus::context::ChainContext;
use ethereum_consensus::preset::minimal::PRESET;
use ethereum_consensus::types::{Address, H256, U64};
use ethereum_light_client_types::consensus::ConsensusUpdateInfo;
use ethereum_light_client_verifier::consensus::test_utils::{
    gen_light_client_update_with_params, MockSyncCommitteeManager,
};
use ethereum_light_client_verifier::context::{Fraction, LightClientContext};
use ethereum_light_client_verifier::updates::ExecutionUpdateInfo;
use hex_literal::hex;

use crate::client_state::ClientState;
use crate::consensus_state::ConsensusState;
use crate::misc::to_lc_types_height;
use ethereum_consensus::beacon::Slot;
use ethereum_light_client_types::consensus::TrustedSyncCommittee;
use light_client::types::{Height, Time};
use std::time::Duration;

/// Sync committee size from minimal preset
pub const SYNC_COMMITTEE_SIZE: usize = PRESET.SYNC_COMMITTEE_SIZE;

/// Type alias for test client state
pub type TestClientState = ClientState<SYNC_COMMITTEE_SIZE>;

/// Type alias for MockSyncCommitteeManager with minimal preset
pub type TestMockSyncCommitteeManager = MockSyncCommitteeManager<SYNC_COMMITTEE_SIZE>;

/// Default genesis time for tests (2020-01-01 00:00:00 UTC)
pub const DEFAULT_GENESIS_TIME: u64 = 1577836800;

/// Creates a LightClientContext with the given parameters.
pub fn create_light_client_context(genesis_time: u64, current_time: u64) -> LightClientContext {
    // Use a non-zero genesis_validators_root for valid client state
    let genesis_validators_root = H256::from_slice(&[1u8; 32]);
    LightClientContext::new_with_config(
        config::minimal::get_config(),
        genesis_validators_root,
        genesis_time.into(),
        Fraction::new(2, 3).unwrap(),
        current_time.into(),
    )
}

/// Creates a simple LightClientContext with default genesis time.
pub fn create_simple_context(now_secs: u64) -> LightClientContext {
    // Use a non-zero genesis_validators_root for valid client state
    let genesis_validators_root = H256::from_slice(&[1u8; 32]);
    LightClientContext::new_with_config(
        config::minimal::get_config(),
        genesis_validators_root,
        Default::default(),
        Fraction::new(2, 3).unwrap(),
        now_secs.into(),
    )
}

/// Converts verifier's ConsensusUpdateInfo to types' ConsensusUpdateInfo.
pub fn to_consensus_update_info<const SYNC_COMMITTEE_SIZE: usize>(
    consensus_update: ethereum_light_client_verifier::updates::ConsensusUpdateInfo<
        SYNC_COMMITTEE_SIZE,
    >,
) -> ConsensusUpdateInfo<SYNC_COMMITTEE_SIZE> {
    ConsensusUpdateInfo {
        attested_header: consensus_update.light_client_update.attested_header,
        next_sync_committee: consensus_update.light_client_update.next_sync_committee,
        finalized_header: consensus_update.light_client_update.finalized_header,
        sync_aggregate: consensus_update.light_client_update.sync_aggregate,
        signature_slot: consensus_update.light_client_update.signature_slot,
        finalized_execution_root: consensus_update.finalized_execution_root,
        finalized_execution_branch: consensus_update.finalized_execution_branch,
    }
}

/// Test fixture containing common test data.
pub struct TestFixture {
    pub scm: TestMockSyncCommitteeManager,
    pub ctx: LightClientContext,
    pub period_1: U64,
    pub base_signature_slot: U64,
    pub base_attested_slot: U64,
    pub base_finalized_epoch: U64,
}

impl TestFixture {
    /// Creates a new test fixture with default parameters.
    pub fn new() -> Self {
        Self::with_genesis_time(DEFAULT_GENESIS_TIME)
    }

    /// Creates a new test fixture with custom genesis time.
    pub fn with_genesis_time(genesis_time: u64) -> Self {
        let current_time = genesis_time + 100_000;
        let scm = MockSyncCommitteeManager::new(1, 4);
        let ctx = create_light_client_context(genesis_time, current_time);

        let period_1 = U64(1) * ctx.slots_per_epoch() * ctx.epochs_per_sync_committee_period();
        let base_signature_slot = period_1 + 11;
        let base_attested_slot = base_signature_slot - 1;
        let base_finalized_epoch = base_attested_slot / ctx.slots_per_epoch();

        Self {
            scm,
            ctx,
            period_1,
            base_signature_slot,
            base_attested_slot,
            base_finalized_epoch,
        }
    }

    /// Creates a new test fixture with simple context (no genesis time offset).
    pub fn simple(now_secs: u64) -> Self {
        let scm = MockSyncCommitteeManager::new(1, 4);
        let ctx = create_simple_context(now_secs);

        let period_1 = U64(1) * ctx.slots_per_epoch() * ctx.epochs_per_sync_committee_period();
        let base_signature_slot = period_1 + 11;
        let base_attested_slot = base_signature_slot - 1;
        let base_finalized_epoch = base_attested_slot / ctx.slots_per_epoch();

        Self {
            scm,
            ctx,
            period_1,
            base_signature_slot,
            base_attested_slot,
            base_finalized_epoch,
        }
    }

    /// Gets the current sync committee (period 1).
    pub fn current_sync_committee(
        &self,
    ) -> &ethereum_light_client_verifier::consensus::test_utils::MockSyncCommittee<
        SYNC_COMMITTEE_SIZE,
    > {
        self.scm.get_committee(1)
    }

    /// Gets the next sync committee (period 2).
    pub fn next_sync_committee(
        &self,
    ) -> &ethereum_light_client_verifier::consensus::test_utils::MockSyncCommittee<
        SYNC_COMMITTEE_SIZE,
    > {
        self.scm.get_committee(2)
    }

    /// Generates a light client update with default parameters.
    pub fn gen_update(
        &self,
        execution_state_root: H256,
        execution_block_number: u64,
    ) -> (
        ethereum_light_client_verifier::updates::ConsensusUpdateInfo<SYNC_COMMITTEE_SIZE>,
        ExecutionUpdateInfo,
    ) {
        gen_light_client_update_with_params::<SYNC_COMMITTEE_SIZE, _>(
            &self.ctx,
            self.base_signature_slot,
            self.base_attested_slot,
            self.base_finalized_epoch,
            execution_state_root,
            execution_block_number.into(),
            self.current_sync_committee(),
            self.next_sync_committee(),
            true,
            PRESET.SYNC_COMMITTEE_SIZE,
        )
    }

    /// Builds a `ConsensusState` whose `current`/`next` sync committee aggregate
    /// pubkeys are the fixture's current/next committees (the common test case).
    pub fn consensus_state(
        &self,
        slot: Slot,
        storage_root: H256,
        timestamp: Time,
    ) -> ConsensusState {
        ConsensusState {
            slot,
            storage_root,
            timestamp,
            current_sync_committee: self
                .current_sync_committee()
                .to_committee()
                .aggregate_pubkey
                .clone(),
            next_sync_committee: self
                .next_sync_committee()
                .to_committee()
                .aggregate_pubkey
                .clone(),
        }
    }

    /// Builds a `TrustedSyncCommittee` backed by the fixture's current committee.
    pub fn trusted_from_current(
        &self,
        height: Height,
        is_next: bool,
    ) -> TrustedSyncCommittee<SYNC_COMMITTEE_SIZE> {
        TrustedSyncCommittee {
            height: to_lc_types_height(height),
            sync_committee: self.current_sync_committee().to_committee(),
            is_next,
        }
    }
}

impl Default for TestFixture {
    fn default() -> Self {
        Self::new()
    }
}

/// Test account proof data from real Ethereum state.
pub mod account_proof {
    use super::*;

    /// Returns a valid account proof for testing.
    pub fn get_proof() -> Vec<Vec<u8>> {
        vec![
            hex!("f901d180a09199d4ddc4f618c0df40c0e1e09eaf2394cd21d566d841b654f3f268196922d0a0bac36050a7d1931b8d6f027075410a85587c649f2d0b30e8ffe967cb3329314ca03919f1f0815704a954616d26504c9201132454a1c0023252294c1abbe0fab26fa0e72e174077c047357c47cba596110765043277d24c55c787ecb164e33a7f1aa5a0de86ea5531307567132648d5c7956cb6082d6803f3dbc9e16b2dd20b320ca93aa0c2c799b60a0cd6acd42c1015512872e86c186bcf196e85061e76842f3b7cf860a088126df40baa53d4d60c0e2a004b6ee8506f131573c750649380e74662093855a02e0d86c3befd177f574a20ac63804532889077e955320c9361cd10b7cc6f5809a0c326f61dd1e74e037d4db73aede5642260bf92869081753bbace550a73989aeda069d63e492e4c3aa54393df9bc12809c9bfc6482b3feb16f2877d7a3e6857d94780a029087b3ba8c5129e161e2cb956640f4d8e31a35f3f133c19a1044993def98b61a08d65cbe14c995d8fe7c7343e9aa31efc7dd81acb0ee940ee565613d8bbbbaa02a0bb12ddf18cf418b9bb5164d2c0caad9e4a29bdca8f1a0c9ed16dd8095f8792fba0144540d36e30b250d25bd5c34d819538742dc54c2017c4eb1fabb8e45f72759180").to_vec(),
            hex!("f8518080a0b595706019b55ae9c4784db71e12bc68d3c991fc1277327e8d63014d10137f7b8080808080808080a04e41195493413c0bbe1fd524bbac490ed81e002fbf4d3d769e0be3452466de0c8080808080").to_vec(),
            hex!("f869a020fff6b964c3925a3b7475bdd2ad96660593de57a6a55a3ef0c82303af814889b846f8440180a02988bb89d212527a6054fec481672b5cdd01bdf7287129442e82bb7569a412f9a0cf76e7c6fa61cca89fee643691266bb1f2721c2d2eeb3063a5e545560abc2b7a").to_vec(),
        ]
    }

    /// Returns the state root corresponding to the test account proof.
    pub fn get_state_root() -> H256 {
        H256::from_slice(&hex!(
            "6a3c41347943fdeab40fb6f0cff088bc81032c86a22b69c67c83b79b72cbb0b4"
        ))
    }

    /// Returns the IBC address corresponding to the test account proof.
    pub fn get_address() -> Address {
        Address(hex!("12496c9aa0e6754c897ca88c1d53fea9b19b8aff"))
    }

    /// Returns the storage root corresponding to the test account proof.
    pub fn get_storage_root() -> H256 {
        H256::from_slice(&hex!(
            "2988BB89D212527A6054FEC481672B5CDD01BDF7287129442E82BB7569A412F9"
        ))
    }
}

/// Creates a test client state with the given context parameters.
pub fn create_test_client_state_from_ctx(ctx: &LightClientContext) -> TestClientState {
    use ethereum_light_client_verifier::context::ConsensusVerificationContext;

    TestClientState {
        genesis_validators_root: ctx.genesis_validators_root(),
        min_sync_committee_participants: 1u64.into(),
        genesis_time: ctx.genesis_time(),
        fork_parameters: ctx.fork_parameters().clone(),
        seconds_per_slot: PRESET.SECONDS_PER_SLOT,
        slots_per_epoch: PRESET.SLOTS_PER_EPOCH,
        epochs_per_sync_committee_period: PRESET.EPOCHS_PER_SYNC_COMMITTEE_PERIOD,
        ibc_address: account_proof::get_address(),
        ibc_commitments_slot: H256::from_slice(&[2u8; 32]),
        trust_level: Fraction::new(2, 3).unwrap(),
        trusting_period: Duration::from_secs(60 * 60 * 24 * 7), // 1 week
        max_clock_drift: Duration::from_secs(60 * 10),          // 10 minutes
        latest_execution_block_number: 1u64.into(),
        ..Default::default()
    }
}

/// Fulu -> Gloas transition data built by the shared relay helpers
/// (ethereum-light-client-types `e2e/client`) against a devnet: minimal preset, every fork at
/// epoch 0 except Gloas at epoch 8, i.e. the first slot of sync committee period 1.
///
/// - the trusted state is finalized in Fulu (period 0, slot 0)
/// - `period_1_header`: the period 1 update, attested slot 80 / finalized slot 64 (Gloas),
///   execution block 63 verified via its RLP header
/// - `period_2_header`: the period 2 update, attested slot 144 / finalized slot 128,
///   execution block 127
/// - `steady_header`: a finality update inside period 2, attested slot 160 / finalized slot 144,
///   execution block 143
pub mod gloas_devnet {
    use super::*;
    use crate::header::{decode_header, Header};
    use ethereum_elc_proto::ibc::lightclients::ethereum::v1::{
        ClientState as RawClientState, ConsensusState as RawConsensusState,
    };
    use prost::Message;

    pub const PERIOD_1_FINALIZED_SLOT: u64 = 64;
    pub const PERIOD_1_EXECUTION_BLOCK_NUMBER: u64 = 63;
    pub const PERIOD_2_FINALIZED_SLOT: u64 = 128;
    pub const PERIOD_2_EXECUTION_BLOCK_NUMBER: u64 = 127;
    pub const STEADY_FINALIZED_SLOT: u64 = 144;
    pub const STEADY_EXECUTION_BLOCK_NUMBER: u64 = 143;

    /// unix seconds of the execution blocks described by the headers. For Gloas this is the
    /// timestamp of the bid's parent block, so it is earlier than the finalized slot itself.
    pub const PERIOD_1_HEADER_TIMESTAMP: u64 = 1790388357;
    pub const PERIOD_2_HEADER_TIMESTAMP: u64 = 1790388741;
    pub const STEADY_HEADER_TIMESTAMP: u64 = 1790388837;
    /// host time when the headers were built (at or after the signature slot)
    pub const PERIOD_1_NOW: u64 = 1790388584;
    pub const PERIOD_2_NOW: u64 = 1790388953;
    pub const STEADY_NOW: u64 = 1790388955;

    /// ibc.lightclients.ethereum.v1.ClientState
    const CLIENT_STATE: &str = concat!(
        "0a2083431ec7fcf92cfc44947fc0418e831c25e1d0806590231c439830db7ad54fda1001188bcedcd506229e010a0400",
        "000001120e0a04010000011a0608691036183712140a04020000011a0c08691036183720192812301612140a04030000",
        "011a0c08691036183720192812301612140a04040000011a0c08691036183720192822302612150a04050000011a0d08",
        "a9011056185720192822302612150a04060000011a0d08a9011056185720192822302612160a040700000110081a0c08",
        "df0510811718821738a816280630083808421442424242424242424242424242424242424242424a201ee222554989dd",
        "a120e26ecacf756fe1235cd8d726706b57517715dde4f0c9005204080210035a040880f5246202083c6801"
    );

    /// ibc.lightclients.ethereum.v1.ConsensusState finalized in Fulu
    const CONSENSUS_STATE: &str = concat!(
        "122000000000000000000000000000000000000000000000000000000000000000001a06088bcedcd5062230b70f6572",
        "399b5fe8fc517c9a0a08659255106251b4e5d0d7d974f3509a37bf49011bfabe9bceb8c9f73e5a71434c2d762a30b70f",
        "6572399b5fe8fc517c9a0a08659255106251b4e5d0d7d974f3509a37bf49011bfabe9bceb8c9f73e5a71434c2d76"
    );

    /// ibc.lightclients.ethereum.v1.Header: the period 1 update (Fulu -> Gloas)
    const PERIOD_1_HEADER: &str = concat!(
        "0af90c0a0012f20c0a30925b1fb57c06b5668567bd5aa196531032d6f8918dd4f702017c11b59288e3bdb98e3820ac22",
        "780f73580a4119de4bbc0a3088c141df77cd9d8d7a71a75c826c41a9c9f03c6ee1b180f3e7852f6a280099ded351b58d",
        "66e653af8e42816a4d8f532e0a30a4855c83d868f772a579133d9f23818008417b743e8447e235d8eb78b1d8f8a9f63f",
        "98c551beb7de254400f89592314d0a30872c61b4a7f8510ec809e5b023f5fdda2105d024c470ddbbeca4bc74e8280af0",
        "d178d749853e8f6a841083ac1b4db98f0a308dde8306920812b32def3b663f7c540b49180345d3bcb8d3770790b7dc80",
        "030ebc06497feebd1bcf017d918f00bfa88f0a30b245d63d3f9d8ea1807a629fcb1b328cb4d542f35a3d5bc478be0df3",
        "89dddd712fc4c816ba3fede9a96320ae6b24a7d80a30a4855c83d868f772a579133d9f23818008417b743e8447e235d8",
        "eb78b1d8f8a9f63f98c551beb7de254400f89592314d0a309648b83a4f09b4ca2021f0c193c5c41df1465715761bca52",
        "671ca790a3e92d67686b97b3d54c6110409779df887bd9c60a30af344fce60dbd5fb850070e6e76a065e1a32485245ef",
        "4f413135a86ae703da88407c5d01c71f6bb06a151ff96cca71910a30a98ed496c2f464226500a6ce04602ff9ef133ed6",
        "316f372f6c744aee165149f7e578b12780e0eacec307ae6907351d990a30ae4d49364e4a36760cc74a675500055b9aed",
        "99bc19d31abb953ea156bb5a76dcf36769d15341b850114a30ffc80577800a30aecc56f2b1c4011d450214d3e1254479",
        "d583a6a5c2c06fbc049512731f76227d140df9f36a3f76b4ccb4df13424035730a30aef9162ee6f29ee82fbfe387756d",
        "84f9ac472eb8709217aaf28f5ef0ea273f6210e531496470b30d2b7747216e3672d50a30b8cd1cef89aa1567a6058957",
        "442a698cf1b267130606f749451152959a5dfb50d243890d4adc2c3309f7696d54af12600a30a34febc12af07316580b",
        "480364f90a76313ccce7927bbe263e27ea270853b02ad4d1428caf55363f3ebebac622cb9fd60a308a0d241955104bed",
        "acb3b829162f2b457915c2beb9018ede8ef8ea80f401b471c42354358da9e62b51c38d54263a78a90a308826e820179f",
        "d321819e78ffee16f50ac528db2da71ad8c269f60b878bc4887c79c0545b3d750e86e490d5ba9083cb700a309314c6de",
        "0386635e2799af798884c2ea09c63b9f079e572acc00b06a7faccce501ea4dfc0b1a23b8603680a5e34813270a30a485",
        "5c83d868f772a579133d9f23818008417b743e8447e235d8eb78b1d8f8a9f63f98c551beb7de254400f89592314d0a30",
        "96746aaba64dc87835ba709332f4d5d7837ada092b439c49d251aecf92aab5dc132e917bf6f59799bc093f976a7bc021",
        "0a30ae241af60691fda1cf8ca44d49573c55818c53b6141800cca2d488b9a3fba71c0f869179fff50c084657831fbeb4",
        "2bf40a3096746aaba64dc87835ba709332f4d5d7837ada092b439c49d251aecf92aab5dc132e917bf6f59799bc093f97",
        "6a7bc0210a30b08f7feb86786c37661afb9951a959c9b465fd11ca98fcbc908fcf49144084051f6c363e2eb4459da2c2",
        "d03d841756920a30a1c76af1545d7901214bb6be06be5d9e458f8e989c19373a920f0018327c83982f6a2ac138260b8d",
        "ef732cb366411ddc0a308dd74e1bb5228fc1fca274fda02b971c1003a4f409bbdfbcfec6426bf2f52addcbbebccdbf45",
        "eee6ae11eb5b5ee7244d0a3094f0c8535601596eb2165adb28ebe495891a3e4ea77ef501e7790cccb281827d377a5a8d",
        "4c200e3595d3f38f8633b4800a30ac9f4df3f20a16a9fefad08817fcbc9a6ee17f7512db006414b4aa6f234c2313585e",
        "f72c5776df55fa6284af4bc3f6310a30b89bebc699769726a318c8e9971bd3171297c61aea4a6578a7a4f94b547dcba5",
        "bac16a89108b6b6a1fe3695d1a874a0b0a308f467e5723deac7659e1ca273e28410cbaa6d495ab66ae77014f4cd21c64",
        "b6b5ab9987c9b5537fe0279bd063fe609be70a30b89bebc699769726a318c8e9971bd3171297c61aea4a6578a7a4f94b",
        "547dcba5bac16a89108b6b6a1fe3695d1a874a0b0a30ae4d49364e4a36760cc74a675500055b9aed99bc19d31abb953e",
        "a156bb5a76dcf36769d15341b850114a30ffc80577800a3086a73886aa0114bbdbba346cb7c07376c81b549a4802c24d",
        "98ebbc54a6a1b5d2ac874ef657cfb27c3644fcb85f97a2b51230b70f6572399b5fe8fc517c9a0a08659255106251b4e5",
        "d0d7d974f3509a37bf49011bfabe9bceb8c9f73e5a71434c2d76180112f9170a6a085010301a20be0107822dc0f14e1e",
        "c322767ba969e223fdd8fe0a753f55c75cf04b22bb61fe22202e5586e3632692d4108a50ba73037bd7c0f68c4cf5fb6f",
        "867e73cc178840bc0f2a20f4080ae3f154db2d46bb3bc1450ee01a3f077c21a43339b6f6f1305bf3467fb712f20c0a30",
        "a4855c83d868f772a579133d9f23818008417b743e8447e235d8eb78b1d8f8a9f63f98c551beb7de254400f89592314d",
        "0a30ab0bdda0f85f842f431beaccf1250bf1fd7ba51b4100fd64364b6401fda85bb0069b3e715b58819684e7fc0b10a7",
        "2a340a30872c61b4a7f8510ec809e5b023f5fdda2105d024c470ddbbeca4bc74e8280af0d178d749853e8f6a841083ac",
        "1b4db98f0a309977f1c8b731a8d5558146bfb86caea26434f3c5878b589bf280a42c9159e700e9df0e4086296c20b011",
        "d2e78c27d3730a3094f0c8535601596eb2165adb28ebe495891a3e4ea77ef501e7790cccb281827d377a5a8d4c200e35",
        "95d3f38f8633b4800a309977f1c8b731a8d5558146bfb86caea26434f3c5878b589bf280a42c9159e700e9df0e408629",
        "6c20b011d2e78c27d3730a30b7e6e187ed813d950a9a17d1e70c03e4de2903596c4c5ff326848515c985deee38198efe",
        "bc265300cd4f1d6bd7b5d2640a3086a73886aa0114bbdbba346cb7c07376c81b549a4802c24d98ebbc54a6a1b5d2ac87",
        "4ef657cfb27c3644fcb85f97a2b50a30a9cf360aa15fb1d1d30ee2b578dc5884823c19661886ae8b892775ccb3bd96b7",
        "d7345569a2aa0b14e4d015c54a6a0c540a30872c61b4a7f8510ec809e5b023f5fdda2105d024c470ddbbeca4bc74e828",
        "0af0d178d749853e8f6a841083ac1b4db98f0a308dde8306920812b32def3b663f7c540b49180345d3bcb8d3770790b7",
        "dc80030ebc06497feebd1bcf017d918f00bfa88f0a309648b83a4f09b4ca2021f0c193c5c41df1465715761bca52671c",
        "a790a3e92d67686b97b3d54c6110409779df887bd9c60a30b397692ccbf442bfe078174c85dbad7fd605e4ff1caf2904",
        "b31e4a4c79d6444813ad9b2093ac8fbd4dd59ec7a4c8c0060a30876dd4705157eb66dc71bc2e07fb151ea53e1a62a0bb",
        "980a7ce72d15f58944a8a3752d754f52f4a60dbfc7b18169f2680a30903e2989e7442ee0a8958d020507a8bd985d3974",
        "f5e8273093be00db3935f0500e141b252bd09e3728892c7a8443863c0a30b08f7feb86786c37661afb9951a959c9b465",
        "fd11ca98fcbc908fcf49144084051f6c363e2eb4459da2c2d03d841756920a30b9d1d914df3d4565465c3fd52b5b96e6",
        "37f9980570cabf5b5d4aadf5a329ac36ad672819d997e735f5052e28b1f0c1040a30ae241af60691fda1cf8ca44d4957",
        "3c55818c53b6141800cca2d488b9a3fba71c0f869179fff50c084657831fbeb42bf40a309648b83a4f09b4ca2021f0c1",
        "93c5c41df1465715761bca52671ca790a3e92d67686b97b3d54c6110409779df887bd9c60a30a9cf360aa15fb1d1d30e",
        "e2b578dc5884823c19661886ae8b892775ccb3bd96b7d7345569a2aa0b14e4d015c54a6a0c540a309893413c00283a3f",
        "9ed9fd9845dda1cea38228d22567f9541dccc357e54a2d6a6e204103c92564cbc05f4905ac7c493a0a30a4855c83d868",
        "f772a579133d9f23818008417b743e8447e235d8eb78b1d8f8a9f63f98c551beb7de254400f89592314d0a3081283b7a",
        "20e1ca460ebd9bbd77005d557370cabb1f9a44f530c4c4c66230f675f8df8b4c2818851aa7d77a80ca5a4a5e0a30ab0b",
        "dda0f85f842f431beaccf1250bf1fd7ba51b4100fd64364b6401fda85bb0069b3e715b58819684e7fc0b10a72a340a30",
        "81054bd51ce57a8415f0c8e0f2fbf94f5a8464552baa33263c20a4da062e5ed994a4d32c171106d2008cd063f48f6fe2",
        "0a30af344fce60dbd5fb850070e6e76a065e1a32485245ef4f413135a86ae703da88407c5d01c71f6bb06a151ff96cca",
        "71910a30a98ed496c2f464226500a6ce04602ff9ef133ed6316f372f6c744aee165149f7e578b12780e0eacec307ae69",
        "07351d990a30af344fce60dbd5fb850070e6e76a065e1a32485245ef4f413135a86ae703da88407c5d01c71f6bb06a15",
        "1ff96cca71910a30aef9162ee6f29ee82fbfe387756d84f9ac472eb8709217aaf28f5ef0ea273f6210e531496470b30d",
        "2b7747216e3672d50a309243ef5ed3bd28892d1ef4f7aaf29faeb9c0e725673cd38e308bd756f20a9ee09de5cd9822e5",
        "e77bd03b734ef8a926950a30aec922bd7a9b7b1dc21993133b586b0c3041c1e2e04b513e862227b9d7aecaf9444222f7",
        "e78282a449622ffc6278915d0a309698d9519a02b64f230e5a2520401799c2ca7d69ab23a6d9817943147264bf00d409",
        "264b928718245efff4f7ee97dd5c12308de62a754de8f022fc3b1cb9dfa920891b4e2903c1712d438e277bbeab30e162",
        "8cb1e7b5ce1bcd51bbdb27b883c32ec81a20103fc06b3954aaffbe9c885de8386f6532aa4d6e78364f59a60cf1ef286c",
        "a06f1a2065d2315367f93be21dcedb8c08575be3421a4b7b405f5dac1ce7234bc54b0cae1a20afddfacdc8ec21b743c2",
        "95755ea8c298f7565519420251330d3a07691e29270b1a20b580979a0a2ec68f5ace0f3ca5c9617d54d2269ddc422993",
        "5a983f32eac4ee461a207fd753d1b6d6f2454d8f9ed591125e0df08543a3882d71773c4ae4ff81ad26991a209efde052",
        "aa15429fae05bad4d0b1d7c64da64d03d7a1854a588c2cb8430c0d301a20000000000000000000000000000000000000",
        "00000000000000000000000000001a20cb5c558204934516989c4b3f30c47086abf3e99f4f55bf2570e9c2a41d721ac1",
        "1a20bc582ddeda554345e7ccf4027b20a36181073a628cb47b8d71339e2c26a5eadc1a200b27b76a0000000000000000",
        "00000000000000000000000000000000000000001a20ffffffffff3f0000000000000000000000000000000000000000",
        "000000000000226a084010141a2019a58e83bb1f522f1704d0233846ec8e8a324d4bfad4e10cfd9cdd06474b6ee62220",
        "8a87739bb8185deb280742e9c01abbf6eaecb2c2a001d66c419054d8de690cc52a20697ff33d780dd6f93f9cc4e98280",
        "c5d5102e8ca3dc1503bdd99f80c8859870602a2008000000000000000000000000000000000000000000000000000000",
        "000000002a205ea4d6f8a2fcc6da4fff76233882cefdd00928be00a501b56b4c3cdfd860470f2a20e7fca9ce847d606d",
        "5882c0b0ad27cf0618b238a1131b96ca5785efe78e54c6292a2030e0f446117923223c47c9af25114548fab6f125f2ff",
        "10f223ac7d2d4d767dd92a20c4ba4ec5993d08f36c88b7dc4dc0673e15821837b224a92ee27b9b676ca77f792a20bd0e",
        "459242d2f31bc64e6d7bf4d5b127ba5a6a14849d146374607a80ea5fe7ac2a20bc582ddeda554345e7ccf4027b20a361",
        "81073a628cb47b8d71339e2c26a5eadc2a200b27b76a0000000000000000000000000000000000000000000000000000",
        "00002a20ffffffffff3f00000000000000000000000000000000000000000000000000003220219034514b5fec562da8",
        "d43d228f98f5f5e1a5ea98588536eb006716dd0745e23a20f2bebec23742609757380f90c5c98f80abc9fd27a6555679",
        "9391efed5f8da9343a20ff0f0000000000000000000000000000000000000000000000000000000000003a208e742f55",
        "52fe005d58a4676d4c9709ac5cba3fc36cb99e387dcb2414edf532523a20f5a5fd42d16a20302798ef6ed309979b4300",
        "3d2320d9f0e8ea9831a92759fb4b3a20ecd4a878b6c7bcf6438295bf67c723ab129332cad5125f5c8de575a63aa3a826",
        "3a2020dec36c38fcd991ea6c84a54d4efcb00810c723521821683462b14d22388fe53a20c78009fdf07fc56a11f12237",
        "0658a353aaa542ed63e44c4bc15ff4cd105ab33c3a200000000000000000000000000000000000000000000000000000",
        "0000000000003a209bf9c82f97afd081146d289c72bea5c27e42d048f327c2fee9f679ed9196ba433a201f778edba610",
        "d3f6166dd9f0f7003b1b53ad2df9d4b61ccfae89aad8262a5f353a20ff1f000000000000000000000000000000000000",
        "00000000000000000000000042680a04ffffffff1260940b9b60df0c03b7f5d25175ef3afd8d2b0360a5b77e2b9c939b",
        "659092087426823ce67b9c0224cec827676a56eae230086694e343b3710dd81cab73065fdb8984c3badeda28aa7f4f25",
        "dad81e7f9b0dd4d9187dfb3bcec57df55676390aabab48511ac3050a20882830f03e1277d19da2c9fcc19e0ab4c82657",
        "09c2d812e0b22f19f9e57ba76e183f2a20219034514b5fec562da8d43d228f98f5f5e1a5ea98588536eb006716dd0745",
        "e23afa04f90277a0f94fca4f4dbe891cc5c771e564279893e18f88ecd010733588a4d07437bbf058a01dcc4de8dec75d",
        "7aab85b567b6ccd41ad312451b948a7413f0a142fd40d4934794a89f47c6b463f74d87572b058427da0a13ec5425a088",
        "2830f03e1277d19da2c9fcc19e0ab4c8265709c2d812e0b22f19f9e57ba76ea056e81f171bcc55a6ff8345e692c0f86e",
        "5b48e01b996cadc001622fb5e363b421a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b4",
        "21b901000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000803f84047bd4ee80846ab7288599d883011106846765746888676f31",
        "2e32362e35856c696e7578a0a35779dc6280fb50730153017b4dae8105922f98ae5a40d6ed980e4fc6a0843188000000",
        "000000000083036461a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b4218080a03d8b0f",
        "14468ed68b3597f793850d0dace5a076a89c4a3c9c09094d76b80219b3a0e3b0c44298fc1c149afbf4c8996fb92427ae",
        "41e4649b934ca495991b7852b8552283050ade04f9025bf90191a0040af9e581aa6a2e51fcc39dfc0b8ab6c4ce2d6d28",
        "13258d608e63adfd410fc1a0402bc96fafd923b6e211f3d1b39bcfc62c58ead5e880e7e3dbb03334cee54f7ea0d719b3",
        "956dc6213a38d6101f3593d89048b76d7e0dc6545e7c74b64c345ad45ca054b58ab2ec368affdc4ac037a979e21493cc",
        "bbe96ea2f589a5199fea90ed04e6a0083c0807a7062baeb2add7696adc8233910d7eec41e9e5a472e0df2a2f0f6ffe80",
        "80a06139222a2cd3ea3efcacd30b9732ce8f660789c25d66afd08cf5f48aafe12598a06fd41477d5a52cccfefbd6fb90",
        "a0c2d2daee3c978ed0886843674086058a3d16a00267cd923c96bc080f1d05bb0b2c64c4a566d9ecc05c0cc5539f8a28",
        "3f5009bea099293f5b3f13831b33cd11b9ca7d16a4848a1e5e1706a2507353e1b358f506d7a0134ff8decf6cb8398fdd",
        "3814418e383904c943c61199d3d3ca9edb74b78cf698a083e7d022eb41ca41a4f08c43b65cef2c9fe21e4774ee62795e",
        "fbec70aaf27f108080a0bcb4bb98765ceafa999c608200fccd2926dd7b7b53dd0a32f356e65fcf0c523980f851808080",
        "a08ee428406349a6c8a82cc7e36560d6694a8700306bebeb20bae8d7d049047cf080a0b62534d9aee474a3fedb98d1ac",
        "e279697d94ae45fdfc8157c00ff6090ccf83128080808080808080808080f872a0202a47fc6863b89a6b51890ef3c155",
        "0d560886c027141d2058ba1e2d4c66d99ab84ff84d808906f05b59d3b2000000a09c19f3c29e89af5150f05e752f7343",
        "0d0b3492d10e0d0057117ab7d0c01a6366a02034f79e0e33b0ae6bef948532021baceb116adf2616478703bec6b17329",
        "f1cc12209c19f3c29e89af5150f05e752f73430d0b3492d10e0d0057117ab7d0c01a6366"
    );

    /// ibc.lightclients.ethereum.v1.Header: the period 2 update (Gloas -> Gloas)
    const PERIOD_2_HEADER: &str = concat!(
        "0afb0c0a02104012f20c0a30a4855c83d868f772a579133d9f23818008417b743e8447e235d8eb78b1d8f8a9f63f98c5",
        "51beb7de254400f89592314d0a30ab0bdda0f85f842f431beaccf1250bf1fd7ba51b4100fd64364b6401fda85bb0069b",
        "3e715b58819684e7fc0b10a72a340a30872c61b4a7f8510ec809e5b023f5fdda2105d024c470ddbbeca4bc74e8280af0",
        "d178d749853e8f6a841083ac1b4db98f0a309977f1c8b731a8d5558146bfb86caea26434f3c5878b589bf280a42c9159",
        "e700e9df0e4086296c20b011d2e78c27d3730a3094f0c8535601596eb2165adb28ebe495891a3e4ea77ef501e7790ccc",
        "b281827d377a5a8d4c200e3595d3f38f8633b4800a309977f1c8b731a8d5558146bfb86caea26434f3c5878b589bf280",
        "a42c9159e700e9df0e4086296c20b011d2e78c27d3730a30b7e6e187ed813d950a9a17d1e70c03e4de2903596c4c5ff3",
        "26848515c985deee38198efebc265300cd4f1d6bd7b5d2640a3086a73886aa0114bbdbba346cb7c07376c81b549a4802",
        "c24d98ebbc54a6a1b5d2ac874ef657cfb27c3644fcb85f97a2b50a30a9cf360aa15fb1d1d30ee2b578dc5884823c1966",
        "1886ae8b892775ccb3bd96b7d7345569a2aa0b14e4d015c54a6a0c540a30872c61b4a7f8510ec809e5b023f5fdda2105",
        "d024c470ddbbeca4bc74e8280af0d178d749853e8f6a841083ac1b4db98f0a308dde8306920812b32def3b663f7c540b",
        "49180345d3bcb8d3770790b7dc80030ebc06497feebd1bcf017d918f00bfa88f0a309648b83a4f09b4ca2021f0c193c5",
        "c41df1465715761bca52671ca790a3e92d67686b97b3d54c6110409779df887bd9c60a30b397692ccbf442bfe078174c",
        "85dbad7fd605e4ff1caf2904b31e4a4c79d6444813ad9b2093ac8fbd4dd59ec7a4c8c0060a30876dd4705157eb66dc71",
        "bc2e07fb151ea53e1a62a0bb980a7ce72d15f58944a8a3752d754f52f4a60dbfc7b18169f2680a30903e2989e7442ee0",
        "a8958d020507a8bd985d3974f5e8273093be00db3935f0500e141b252bd09e3728892c7a8443863c0a30b08f7feb8678",
        "6c37661afb9951a959c9b465fd11ca98fcbc908fcf49144084051f6c363e2eb4459da2c2d03d841756920a30b9d1d914",
        "df3d4565465c3fd52b5b96e637f9980570cabf5b5d4aadf5a329ac36ad672819d997e735f5052e28b1f0c1040a30ae24",
        "1af60691fda1cf8ca44d49573c55818c53b6141800cca2d488b9a3fba71c0f869179fff50c084657831fbeb42bf40a30",
        "9648b83a4f09b4ca2021f0c193c5c41df1465715761bca52671ca790a3e92d67686b97b3d54c6110409779df887bd9c6",
        "0a30a9cf360aa15fb1d1d30ee2b578dc5884823c19661886ae8b892775ccb3bd96b7d7345569a2aa0b14e4d015c54a6a",
        "0c540a309893413c00283a3f9ed9fd9845dda1cea38228d22567f9541dccc357e54a2d6a6e204103c92564cbc05f4905",
        "ac7c493a0a30a4855c83d868f772a579133d9f23818008417b743e8447e235d8eb78b1d8f8a9f63f98c551beb7de2544",
        "00f89592314d0a3081283b7a20e1ca460ebd9bbd77005d557370cabb1f9a44f530c4c4c66230f675f8df8b4c2818851a",
        "a7d77a80ca5a4a5e0a30ab0bdda0f85f842f431beaccf1250bf1fd7ba51b4100fd64364b6401fda85bb0069b3e715b58",
        "819684e7fc0b10a72a340a3081054bd51ce57a8415f0c8e0f2fbf94f5a8464552baa33263c20a4da062e5ed994a4d32c",
        "171106d2008cd063f48f6fe20a30af344fce60dbd5fb850070e6e76a065e1a32485245ef4f413135a86ae703da88407c",
        "5d01c71f6bb06a151ff96cca71910a30a98ed496c2f464226500a6ce04602ff9ef133ed6316f372f6c744aee165149f7",
        "e578b12780e0eacec307ae6907351d990a30af344fce60dbd5fb850070e6e76a065e1a32485245ef4f413135a86ae703",
        "da88407c5d01c71f6bb06a151ff96cca71910a30aef9162ee6f29ee82fbfe387756d84f9ac472eb8709217aaf28f5ef0",
        "ea273f6210e531496470b30d2b7747216e3672d50a309243ef5ed3bd28892d1ef4f7aaf29faeb9c0e725673cd38e308b",
        "d756f20a9ee09de5cd9822e5e77bd03b734ef8a926950a30aec922bd7a9b7b1dc21993133b586b0c3041c1e2e04b513e",
        "862227b9d7aecaf9444222f7e78282a449622ffc6278915d0a309698d9519a02b64f230e5a2520401799c2ca7d69ab23",
        "a6d9817943147264bf00d409264b928718245efff4f7ee97dd5c12308de62a754de8f022fc3b1cb9dfa920891b4e2903",
        "c1712d438e277bbeab30e1628cb1e7b5ce1bcd51bbdb27b883c32ec8180112fc170a6b089001101c1a20775550b6145c",
        "175466fc154aec6941503498443728b4bbdc9590ebf7e141fa9c222056a5f95014e34e2b5489a99a3da57789002f50e5",
        "6171407cd2b49a5690eacfac2a2035e46f73981ce7e9de95b0d5bb17c5fe126096e1d6d3a6cbcd87d3023c0b09a212f2",
        "0c0a3081283b7a20e1ca460ebd9bbd77005d557370cabb1f9a44f530c4c4c66230f675f8df8b4c2818851aa7d77a80ca",
        "5a4a5e0a3081283b7a20e1ca460ebd9bbd77005d557370cabb1f9a44f530c4c4c66230f675f8df8b4c2818851aa7d77a",
        "80ca5a4a5e0a30af344fce60dbd5fb850070e6e76a065e1a32485245ef4f413135a86ae703da88407c5d01c71f6bb06a",
        "151ff96cca71910a3091ae4686b0d20470409f020eaca826c3efc6c1926ed25d05e6f0f7916391ec89c2341917277c43",
        "7ac8fffffe94b681110a30872c61b4a7f8510ec809e5b023f5fdda2105d024c470ddbbeca4bc74e8280af0d178d74985",
        "3e8f6a841083ac1b4db98f0a3087c9f7605d07550b46c79add5ea4e39de5014c03833669257bd6666b7ec838f5380010",
        "4779940d8cdd884275a0f6a3ef0a30a34febc12af07316580b480364f90a76313ccce7927bbe263e27ea270853b02ad4",
        "d1428caf55363f3ebebac622cb9fd60a30ac9f4df3f20a16a9fefad08817fcbc9a6ee17f7512db006414b4aa6f234c23",
        "13585ef72c5776df55fa6284af4bc3f6310a30afc0fa2ed6a270de6122a19d4600380b7f9b5e974d16f095f1702f5579",
        "2ecab0128b155a69f17ad64a6de0a7063642ec0a308dd74e1bb5228fc1fca274fda02b971c1003a4f409bbdfbcfec642",
        "6bf2f52addcbbebccdbf45eee6ae11eb5b5ee7244d0a30b8cd1cef89aa1567a6058957442a698cf1b267130606f74945",
        "1152959a5dfb50d243890d4adc2c3309f7696d54af12600a30a4855c83d868f772a579133d9f23818008417b743e8447",
        "e235d8eb78b1d8f8a9f63f98c551beb7de254400f89592314d0a309648b83a4f09b4ca2021f0c193c5c41df146571576",
        "1bca52671ca790a3e92d67686b97b3d54c6110409779df887bd9c60a30963528adb5322c2e2c54dc296ffddd2861bb10",
        "3cbf64646781dfa8a3c2d8a8eda7079d2b3e95600028c44365afbf88790a30963528adb5322c2e2c54dc296ffddd2861",
        "bb103cbf64646781dfa8a3c2d8a8eda7079d2b3e95600028c44365afbf88790a30a9cf360aa15fb1d1d30ee2b578dc58",
        "84823c19661886ae8b892775ccb3bd96b7d7345569a2aa0b14e4d015c54a6a0c540a30925b1fb57c06b5668567bd5aa1",
        "96531032d6f8918dd4f702017c11b59288e3bdb98e3820ac22780f73580a4119de4bbc0a3092977e71396633d442f61e",
        "16a0cfcf8ffad0af93c9f1b7fdf4f7ccb816de052925fc192922d6252d325ef9fa2e0595d20a30876dd4705157eb66dc",
        "71bc2e07fb151ea53e1a62a0bb980a7ce72d15f58944a8a3752d754f52f4a60dbfc7b18169f2680a308dd74e1bb5228f",
        "c1fca274fda02b971c1003a4f409bbdfbcfec6426bf2f52addcbbebccdbf45eee6ae11eb5b5ee7244d0a30afc0fa2ed6",
        "a270de6122a19d4600380b7f9b5e974d16f095f1702f55792ecab0128b155a69f17ad64a6de0a7063642ec0a30ab8d3a",
        "9bcc160e518fac0756d3e192c74789588ed4a2b1debf0c78f78479ca8edb05b12ce21103076df6af4eb8756ff90a30a8",
        "52816b8e463178eea5acebb4b86d0acb6d8c6812cf313296bd271ea4d2fd89d281e5fc296df4df49019169bdf969220a",
        "30ae00fc3de831b09661a0ac02873c45c84cb2b58cffb6430a3f607e4c3fa1e0932397f11307cd169cdc6f79c4635272",
        "600a3080a2be2c7dbce8ddc2eba03522697587c375a5a9e92d4b31ed9e3c34bee047095d93e3c70b1662b3faa301f5b1",
        "9978e50a30876dd4705157eb66dc71bc2e07fb151ea53e1a62a0bb980a7ce72d15f58944a8a3752d754f52f4a60dbfc7",
        "b18169f2680a308f467e5723deac7659e1ca273e28410cbaa6d495ab66ae77014f4cd21c64b6b5ab9987c9b5537fe027",
        "9bd063fe609be70a3087c9f7605d07550b46c79add5ea4e39de5014c03833669257bd6666b7ec838f53800104779940d",
        "8cdd884275a0f6a3ef0a308826e820179fd321819e78ffee16f50ac528db2da71ad8c269f60b878bc4887c79c0545b3d",
        "750e86e490d5ba9083cb700a309698d9519a02b64f230e5a2520401799c2ca7d69ab23a6d9817943147264bf00d40926",
        "4b928718245efff4f7ee97dd5c0a30af344fce60dbd5fb850070e6e76a065e1a32485245ef4f413135a86ae703da8840",
        "7c5d01c71f6bb06a151ff96cca71910a30954eb88ed1207f891dc3c28fa6cfdf8f53bf0ed3d838f3476c0900a61314d2",
        "2d4f0a300da3cd010444dd5183e35a593c123086e5bd64b9a0696875a387fc68cf0c8b9703ccb728b50c601aaf1a82c9",
        "afac9ba05bb73c552987ce3fe6478cd635e7f41a2019344d09af5e30f75df8b3d5b95fa83e304e5f60653ec3dd47b42f",
        "f39208b92e1a206e8171ba2f7b07e982ae2191cd9faff7f4515be11484effca22806cb0f7b10aa1a200aa6bc5901bc45",
        "ba0c554123e71611e1374d11f2651c52b488cf7ebaf77b64ed1a20b580979a0a2ec68f5ace0f3ca5c9617d54d2269ddc",
        "4229935a983f32eac4ee461a204fc2ea9b2063edc7ca06bd4faac466beef5c6a6d5d004fa624721d8e97de0c291a209e",
        "fde052aa15429fae05bad4d0b1d7c64da64d03d7a1854a588c2cb8430c0d301a20000000000000000000000000000000",
        "00000000000000000000000000000000001a2061dd4362432473043d05f97c3e88f34bdea6c449476ba329a4daa08775",
        "36f28a1a2094c527105c2b887b69547d3668951c218e6d269fd0e3708e43b2a00b8aa0f89f1a200b27b76a0000000000",
        "00000000000000000000000000000000000000000000001a20ffffffffff3f0000000000000000000000000000000000",
        "000000000000000000226b08800110261a207c2425fd7b2e54ddfa281bd7b7073cf516c2b93f5e8ef30ba3ab52a715b2",
        "aa2722203c31bf4fce0ca68420267a4e06c16a3591f7283f9a28a19cd7b8e3fb3210f7672a2012e2c092ce78e32c7545",
        "689e52176b9641db945d089c44dcdd3cec69df2b47682a20100000000000000000000000000000000000000000000000",
        "00000000000000002a20ddc3316911233f55f7583ee660917112ce91920d9388a51d1496e041bbfdbae22a200e413a49",
        "32cf6c1725151f37ee1d7c7556dc7b2deeda8016f52f74c56b6601632a207a3a00e1c152acf6f2bb8dfe50f0a0d9ef51",
        "80aa4e77288fd8334bad36d076c02a209ffc618ca8b219896ebbb475866b7af8cd2646aa6e50fd49a9b53037ad735b1f",
        "2a2086aee048e0508c13f2bcfadee006a05ef1d33aa2dc951138fb7af5e17186292c2a2094c527105c2b887b69547d36",
        "68951c218e6d269fd0e3708e43b2a00b8aa0f89f2a200b27b76a00000000000000000000000000000000000000000000",
        "0000000000002a20ffffffffff3f000000000000000000000000000000000000000000000000000032201511f49abdab",
        "2acc342fc1e57d20cccd7cfa320c5557831694d05e9006b807c63a20d6eb55dba348677fc768d016bb1f6b9fa5ddd5b2",
        "bf352d4dd2b096e6691f21083a20ff0f0000000000000000000000000000000000000000000000000000000000003a20",
        "8e742f5552fe005d58a4676d4c9709ac5cba3fc36cb99e387dcb2414edf532523a20f5a5fd42d16a20302798ef6ed309",
        "979b43003d2320d9f0e8ea9831a92759fb4b3a20f8fa7d3ce59188b6040ef812d39f32309009215c54287e95eb43e9d3",
        "c857b1c93a200b18b990823fc6f226b6a5d10c8e68899fafcb3756025edc297913a45de522c13a20c78009fdf07fc56a",
        "11f122370658a353aaa542ed63e44c4bc15ff4cd105ab33c3a2000000000000000000000000000000000000000000000",
        "000000000000000000003a209bf9c82f97afd081146d289c72bea5c27e42d048f327c2fee9f679ed9196ba433a20df6c",
        "801d05ecfdbbff60f42c2f4fca0c10138df69e5be212378ac7977b26430b3a20ff1f0000000000000000000000000000",
        "0000000000000000000000000000000042680a04ffffffff126091f114664c9f0267df9eecfca60f5077603e905a74c7",
        "cee6597febf3f7510a54aeb0048e761d12dd623c660c26e5b1600a833a52c91b198b0b527c7d46034221931fb3145a3e",
        "bcd8e2add6367d7a7dc20f1f147e4b832a86d5853585f35bce134891011ae2050a20882830f03e1277d19da2c9fcc19e",
        "0ab4c8265709c2d812e0b22f19f9e57ba76e187f2a201511f49abdab2acc342fc1e57d20cccd7cfa320c5557831694d0",
        "5e9006b807c63a9905f90296a0c8565c20a45efbdeabe34a05ed86affa94e54bc93cbc938b6fb5a6f118fad794a01dcc",
        "4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d4934794a89f47c6b463f74d87572b058427da0a13",
        "ec5425a0882830f03e1277d19da2c9fcc19e0ab4c8265709c2d812e0b22f19f9e57ba76ea056e81f171bcc55a6ff8345",
        "e692c0f86e5b48e01b996cadc001622fb5e363b421a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc00162",
        "2fb5e363b421b90100000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "00000000000000000000000000000000000000000000000000807f84043641b480846ab72a0599d88301110684676574",
        "6888676f312e32362e35856c696e7578a0205aec02b8330824e639fb76d87fb8989414afb3fcc48cf9de235a9fceed04",
        "1c8800000000000000002fa056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b4218080a079",
        "a14688b28d577f585277d631329a71f311a86d81b12aa92c94b134a8e81211a0e3b0c44298fc1c149afbf4c8996fb924",
        "27ae41e4649b934ca495991b7852b855a0f74cbafae1dd4e32a3b767823eab0f1649628c990ee41a3252720c3e260996",
        "487f2283050ade04f9025bf90191a0040af9e581aa6a2e51fcc39dfc0b8ab6c4ce2d6d2813258d608e63adfd410fc1a0",
        "402bc96fafd923b6e211f3d1b39bcfc62c58ead5e880e7e3dbb03334cee54f7ea0d719b3956dc6213a38d6101f3593d8",
        "9048b76d7e0dc6545e7c74b64c345ad45ca054b58ab2ec368affdc4ac037a979e21493ccbbe96ea2f589a5199fea90ed",
        "04e6a0083c0807a7062baeb2add7696adc8233910d7eec41e9e5a472e0df2a2f0f6ffe8080a06139222a2cd3ea3efcac",
        "d30b9732ce8f660789c25d66afd08cf5f48aafe12598a06fd41477d5a52cccfefbd6fb90a0c2d2daee3c978ed0886843",
        "674086058a3d16a00267cd923c96bc080f1d05bb0b2c64c4a566d9ecc05c0cc5539f8a283f5009bea099293f5b3f1383",
        "1b33cd11b9ca7d16a4848a1e5e1706a2507353e1b358f506d7a0134ff8decf6cb8398fdd3814418e383904c943c61199",
        "d3d3ca9edb74b78cf698a083e7d022eb41ca41a4f08c43b65cef2c9fe21e4774ee62795efbec70aaf27f108080a0bcb4",
        "bb98765ceafa999c608200fccd2926dd7b7b53dd0a32f356e65fcf0c523980f851808080a08ee428406349a6c8a82cc7",
        "e36560d6694a8700306bebeb20bae8d7d049047cf080a0b62534d9aee474a3fedb98d1ace279697d94ae45fdfc8157c0",
        "0ff6090ccf83128080808080808080808080f872a0202a47fc6863b89a6b51890ef3c1550d560886c027141d2058ba1e",
        "2d4c66d99ab84ff84d808906f05b59d3b2000000a09c19f3c29e89af5150f05e752f73430d0b3492d10e0d0057117ab7",
        "d0c01a6366a02034f79e0e33b0ae6bef948532021baceb116adf2616478703bec6b17329f1cc12209c19f3c29e89af51",
        "50f05e752f73430d0b3492d10e0d0057117ab7d0c01a6366"
    );

    /// ibc.lightclients.ethereum.v1.Header: a finality update inside period 2
    const STEADY_HEADER: &str = concat!(
        "0afa0c0a0310800112f20c0a30a4855c83d868f772a579133d9f23818008417b743e8447e235d8eb78b1d8f8a9f63f98",
        "c551beb7de254400f89592314d0a30ab0bdda0f85f842f431beaccf1250bf1fd7ba51b4100fd64364b6401fda85bb006",
        "9b3e715b58819684e7fc0b10a72a340a30872c61b4a7f8510ec809e5b023f5fdda2105d024c470ddbbeca4bc74e8280a",
        "f0d178d749853e8f6a841083ac1b4db98f0a309977f1c8b731a8d5558146bfb86caea26434f3c5878b589bf280a42c91",
        "59e700e9df0e4086296c20b011d2e78c27d3730a3094f0c8535601596eb2165adb28ebe495891a3e4ea77ef501e7790c",
        "ccb281827d377a5a8d4c200e3595d3f38f8633b4800a309977f1c8b731a8d5558146bfb86caea26434f3c5878b589bf2",
        "80a42c9159e700e9df0e4086296c20b011d2e78c27d3730a30b7e6e187ed813d950a9a17d1e70c03e4de2903596c4c5f",
        "f326848515c985deee38198efebc265300cd4f1d6bd7b5d2640a3086a73886aa0114bbdbba346cb7c07376c81b549a48",
        "02c24d98ebbc54a6a1b5d2ac874ef657cfb27c3644fcb85f97a2b50a30a9cf360aa15fb1d1d30ee2b578dc5884823c19",
        "661886ae8b892775ccb3bd96b7d7345569a2aa0b14e4d015c54a6a0c540a30872c61b4a7f8510ec809e5b023f5fdda21",
        "05d024c470ddbbeca4bc74e8280af0d178d749853e8f6a841083ac1b4db98f0a308dde8306920812b32def3b663f7c54",
        "0b49180345d3bcb8d3770790b7dc80030ebc06497feebd1bcf017d918f00bfa88f0a309648b83a4f09b4ca2021f0c193",
        "c5c41df1465715761bca52671ca790a3e92d67686b97b3d54c6110409779df887bd9c60a30b397692ccbf442bfe07817",
        "4c85dbad7fd605e4ff1caf2904b31e4a4c79d6444813ad9b2093ac8fbd4dd59ec7a4c8c0060a30876dd4705157eb66dc",
        "71bc2e07fb151ea53e1a62a0bb980a7ce72d15f58944a8a3752d754f52f4a60dbfc7b18169f2680a30903e2989e7442e",
        "e0a8958d020507a8bd985d3974f5e8273093be00db3935f0500e141b252bd09e3728892c7a8443863c0a30b08f7feb86",
        "786c37661afb9951a959c9b465fd11ca98fcbc908fcf49144084051f6c363e2eb4459da2c2d03d841756920a30b9d1d9",
        "14df3d4565465c3fd52b5b96e637f9980570cabf5b5d4aadf5a329ac36ad672819d997e735f5052e28b1f0c1040a30ae",
        "241af60691fda1cf8ca44d49573c55818c53b6141800cca2d488b9a3fba71c0f869179fff50c084657831fbeb42bf40a",
        "309648b83a4f09b4ca2021f0c193c5c41df1465715761bca52671ca790a3e92d67686b97b3d54c6110409779df887bd9",
        "c60a30a9cf360aa15fb1d1d30ee2b578dc5884823c19661886ae8b892775ccb3bd96b7d7345569a2aa0b14e4d015c54a",
        "6a0c540a309893413c00283a3f9ed9fd9845dda1cea38228d22567f9541dccc357e54a2d6a6e204103c92564cbc05f49",
        "05ac7c493a0a30a4855c83d868f772a579133d9f23818008417b743e8447e235d8eb78b1d8f8a9f63f98c551beb7de25",
        "4400f89592314d0a3081283b7a20e1ca460ebd9bbd77005d557370cabb1f9a44f530c4c4c66230f675f8df8b4c281885",
        "1aa7d77a80ca5a4a5e0a30ab0bdda0f85f842f431beaccf1250bf1fd7ba51b4100fd64364b6401fda85bb0069b3e715b",
        "58819684e7fc0b10a72a340a3081054bd51ce57a8415f0c8e0f2fbf94f5a8464552baa33263c20a4da062e5ed994a4d3",
        "2c171106d2008cd063f48f6fe20a30af344fce60dbd5fb850070e6e76a065e1a32485245ef4f413135a86ae703da8840",
        "7c5d01c71f6bb06a151ff96cca71910a30a98ed496c2f464226500a6ce04602ff9ef133ed6316f372f6c744aee165149",
        "f7e578b12780e0eacec307ae6907351d990a30af344fce60dbd5fb850070e6e76a065e1a32485245ef4f413135a86ae7",
        "03da88407c5d01c71f6bb06a151ff96cca71910a30aef9162ee6f29ee82fbfe387756d84f9ac472eb8709217aaf28f5e",
        "f0ea273f6210e531496470b30d2b7747216e3672d50a309243ef5ed3bd28892d1ef4f7aaf29faeb9c0e725673cd38e30",
        "8bd756f20a9ee09de5cd9822e5e77bd03b734ef8a926950a30aec922bd7a9b7b1dc21993133b586b0c3041c1e2e04b51",
        "3e862227b9d7aecaf9444222f7e78282a449622ffc6278915d0a309698d9519a02b64f230e5a2520401799c2ca7d69ab",
        "23a6d9817943147264bf00d409264b928718245efff4f7ee97dd5c12308de62a754de8f022fc3b1cb9dfa920891b4e29",
        "03c1712d438e277bbeab30e1628cb1e7b5ce1bcd51bbdb27b883c32ec81291080a6b08a001102d1a20b6da032dde827e",
        "9918d8c75eb14dddd5f117affa42156b0d1af8550ee9db36482220fbda13cc758c7bbde64361f887a41a9c8cfde7fa74",
        "d74c62c43f3a20587ee1b42a2083bf4a4473d1f030658f6c0ee132274f66f60b39472566d5c5dd506211712ee5226b08",
        "9001101c1a20775550b6145c175466fc154aec6941503498443728b4bbdc9590ebf7e141fa9c222056a5f95014e34e2b",
        "5489a99a3da57789002f50e56171407cd2b49a5690eacfac2a2035e46f73981ce7e9de95b0d5bb17c5fe126096e1d6d3",
        "a6cbcd87d3023c0b09a22a2012000000000000000000000000000000000000000000000000000000000000002a20ce74",
        "3b4221fbcb342badc437128f8d3c66d71a2593ea0878774e967578b133f42a20c78f7433e93a96722d94a1ea8a4b7f3f",
        "8f321806d0d1ba108fa8cc0b5d74331f2a200857abba57487f34dac1c6fda5434e2d5ec6f903f250fa41c1179ad8a1d7",
        "34982a20657b5a6ea06431c18e9b687cd642c6857f7b0d5c9c097419495b97f8ad206c3b2a2063664ef0be6a2993cc84",
        "8be0bfb98bed61af15bb932b6d8b3f46ef04799ee3bc2a20b18bdaee9fc68f59a60c63c7379a677937481204b911586f",
        "e037b302dc3a81232a200b27b76a000000000000000000000000000000000000000000000000000000002a20ffffffff",
        "ff3f0000000000000000000000000000000000000000000000000000322019344d09af5e30f75df8b3d5b95fa83e304e",
        "5f60653ec3dd47b42ff39208b92e3a2003d60a8bdfd8d80a86776831c8be28265ce23176790c9189f13061002cb32eb1",
        "3a20ff0f0000000000000000000000000000000000000000000000000000000000003a208e742f5552fe005d58a4676d",
        "4c9709ac5cba3fc36cb99e387dcb2414edf532523a20f5a5fd42d16a20302798ef6ed309979b43003d2320d9f0e8ea98",
        "31a92759fb4b3a202d39bc535781a7aad7645ebd151ce52ee3e966e34313623f725bf0dd65fc7b953a200075ebb7c619",
        "ced5f6cbf8e29da7e27f5c338419dc61a447280d891baa1c93a73a20c78009fdf07fc56a11f122370658a353aaa542ed",
        "63e44c4bc15ff4cd105ab33c3a2000000000000000000000000000000000000000000000000000000000000000003a20",
        "9bf9c82f97afd081146d289c72bea5c27e42d048f327c2fee9f679ed9196ba433a20fc73fa109f3f6e33fcef1051220f",
        "aa48901f2dc541bcbfd28aa3a49432bdec663a20ff1f0000000000000000000000000000000000000000000000000000",
        "0000000042680a04ffffffff12608d156975e2e2c837566925143de84b25ef00a72203a4df3620b3140b0e3d8ec4960b",
        "dec19d76dc6c50822730fa3c3b310b908a0402aa6fdc4d881485af751cfacede719f518cd3964bc311677088f305c163",
        "22eff073e663374af8420a015e7048a1011ae5050a20882830f03e1277d19da2c9fcc19e0ab4c8265709c2d812e0b22f",
        "19f9e57ba76e188f012a2019344d09af5e30f75df8b3d5b95fa83e304e5f60653ec3dd47b42ff39208b92e3a9b05f902",
        "98a0d0a6ebb1c6be451bdd7e35a487c182ceb52e62c07f42e3d47c0881fe74250cdca01dcc4de8dec75d7aab85b567b6",
        "ccd41ad312451b948a7413f0a142fd40d4934794a89f47c6b463f74d87572b058427da0a13ec5425a0882830f03e1277",
        "d19da2c9fcc19e0ab4c8265709c2d812e0b22f19f9e57ba76ea056e81f171bcc55a6ff8345e692c0f86e5b48e01b996c",
        "adc001622fb5e363b421a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421b901000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000080818f840425883580846ab72a6599d883011106846765746888676f312e32362e35",
        "856c696e7578a04cb82724badf924a7acf440d31c0d36a2201247a89b487ff2d0055a3475fd856880000000000000000",
        "09a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b4218080a09c998772c1d2a6b235256d",
        "e4b329d5eedf1718b7ea2dfcc49f07f3472f541759a0e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495",
        "991b7852b855a0f74cbafae1dd4e32a3b767823eab0f1649628c990ee41a3252720c3e26099648818f2283050ade04f9",
        "025bf90191a0040af9e581aa6a2e51fcc39dfc0b8ab6c4ce2d6d2813258d608e63adfd410fc1a0402bc96fafd923b6e2",
        "11f3d1b39bcfc62c58ead5e880e7e3dbb03334cee54f7ea0d719b3956dc6213a38d6101f3593d89048b76d7e0dc6545e",
        "7c74b64c345ad45ca054b58ab2ec368affdc4ac037a979e21493ccbbe96ea2f589a5199fea90ed04e6a0083c0807a706",
        "2baeb2add7696adc8233910d7eec41e9e5a472e0df2a2f0f6ffe8080a06139222a2cd3ea3efcacd30b9732ce8f660789",
        "c25d66afd08cf5f48aafe12598a06fd41477d5a52cccfefbd6fb90a0c2d2daee3c978ed0886843674086058a3d16a002",
        "67cd923c96bc080f1d05bb0b2c64c4a566d9ecc05c0cc5539f8a283f5009bea099293f5b3f13831b33cd11b9ca7d16a4",
        "848a1e5e1706a2507353e1b358f506d7a0134ff8decf6cb8398fdd3814418e383904c943c61199d3d3ca9edb74b78cf6",
        "98a083e7d022eb41ca41a4f08c43b65cef2c9fe21e4774ee62795efbec70aaf27f108080a0bcb4bb98765ceafa999c60",
        "8200fccd2926dd7b7b53dd0a32f356e65fcf0c523980f851808080a08ee428406349a6c8a82cc7e36560d6694a870030",
        "6bebeb20bae8d7d049047cf080a0b62534d9aee474a3fedb98d1ace279697d94ae45fdfc8157c00ff6090ccf83128080",
        "808080808080808080f872a0202a47fc6863b89a6b51890ef3c1550d560886c027141d2058ba1e2d4c66d99ab84ff84d",
        "808906f05b59d3b2000000a09c19f3c29e89af5150f05e752f73430d0b3492d10e0d0057117ab7d0c01a6366a02034f7",
        "9e0e33b0ae6bef948532021baceb116adf2616478703bec6b17329f1cc12209c19f3c29e89af5150f05e752f73430d0b",
        "3492d10e0d0057117ab7d0c01a6366"
    );

    fn decode_hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    pub fn client_state() -> TestClientState {
        RawClientState::decode(decode_hex(CLIENT_STATE).as_slice())
            .unwrap()
            .try_into()
            .unwrap()
    }

    pub fn consensus_state() -> ConsensusState {
        RawConsensusState::decode(decode_hex(CONSENSUS_STATE).as_slice())
            .unwrap()
            .try_into()
            .unwrap()
    }

    pub fn period_1_header() -> Header<SYNC_COMMITTEE_SIZE> {
        decode_header(decode_hex(PERIOD_1_HEADER).as_slice()).unwrap()
    }

    pub fn period_2_header() -> Header<SYNC_COMMITTEE_SIZE> {
        decode_header(decode_hex(PERIOD_2_HEADER).as_slice()).unwrap()
    }

    pub fn steady_header() -> Header<SYNC_COMMITTEE_SIZE> {
        decode_header(decode_hex(STEADY_HEADER).as_slice()).unwrap()
    }
}
