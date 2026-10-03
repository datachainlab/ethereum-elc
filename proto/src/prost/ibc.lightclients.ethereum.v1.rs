#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ClientState {
    #[prost(bytes = "vec", tag = "1")]
    pub genesis_validators_root: ::prost::alloc::vec::Vec<u8>,
    #[prost(uint64, tag = "2")]
    pub min_sync_committee_participants: u64,
    #[prost(uint64, tag = "3")]
    pub genesis_time: u64,
    #[prost(message, optional, tag = "4")]
    pub fork_parameters: ::core::option::Option<
        ::ethereum_light_client_proto::ibc::lightclients::ethereum::v1::ForkParameters,
    >,
    #[prost(uint64, tag = "5")]
    pub seconds_per_slot: u64,
    #[prost(uint64, tag = "6")]
    pub slots_per_epoch: u64,
    #[prost(uint64, tag = "7")]
    pub epochs_per_sync_committee_period: u64,
    #[prost(bytes = "vec", tag = "8")]
    pub ibc_address: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes = "vec", tag = "9")]
    pub ibc_commitments_slot: ::prost::alloc::vec::Vec<u8>,
    #[prost(message, optional, tag = "10")]
    pub trust_level: ::core::option::Option<
        ::ethereum_light_client_proto::ibc::lightclients::ethereum::v1::Fraction,
    >,
    #[prost(message, optional, tag = "11")]
    pub trusting_period: ::core::option::Option<
        super::super::super::super::google::protobuf::Duration,
    >,
    #[prost(message, optional, tag = "12")]
    pub max_clock_drift: ::core::option::Option<
        super::super::super::super::google::protobuf::Duration,
    >,
    #[prost(uint64, tag = "13")]
    pub latest_execution_block_number: u64,
    #[prost(message, optional, tag = "14")]
    pub frozen_height: ::core::option::Option<
        ::ibc_proto::ibc::core::client::v1::Height,
    >,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ConsensusState {
    #[prost(uint64, tag = "1")]
    pub slot: u64,
    #[prost(bytes = "vec", tag = "2")]
    pub storage_root: ::prost::alloc::vec::Vec<u8>,
    #[prost(message, optional, tag = "3")]
    pub timestamp: ::core::option::Option<
        super::super::super::super::google::protobuf::Timestamp,
    >,
    #[prost(bytes = "vec", tag = "4")]
    pub current_sync_committee: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes = "vec", tag = "5")]
    pub next_sync_committee: ::prost::alloc::vec::Vec<u8>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Header {
    #[prost(message, optional, tag = "1")]
    pub trusted_sync_committee: ::core::option::Option<
        ::ethereum_light_client_proto::ibc::lightclients::ethereum::v1::TrustedSyncCommittee,
    >,
    #[prost(message, optional, tag = "2")]
    pub consensus_update: ::core::option::Option<
        ::ethereum_light_client_proto::ibc::lightclients::ethereum::v1::ConsensusUpdate,
    >,
    #[prost(message, optional, tag = "3")]
    pub execution_update: ::core::option::Option<
        ::ethereum_light_client_proto::ibc::lightclients::ethereum::v1::ExecutionUpdate,
    >,
    #[prost(message, optional, tag = "4")]
    pub account_update: ::core::option::Option<
        ::ethereum_light_client_proto::ibc::lightclients::ethereum::v1::AccountUpdate,
    >,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FinalizedHeaderMisbehaviour {
    #[prost(string, tag = "1")]
    pub client_id: ::prost::alloc::string::String,
    #[prost(message, optional, tag = "2")]
    pub trusted_sync_committee: ::core::option::Option<
        ::ethereum_light_client_proto::ibc::lightclients::ethereum::v1::TrustedSyncCommittee,
    >,
    #[prost(message, optional, tag = "3")]
    pub consensus_update_1: ::core::option::Option<
        ::ethereum_light_client_proto::ibc::lightclients::ethereum::v1::ConsensusUpdate,
    >,
    #[prost(message, optional, tag = "4")]
    pub consensus_update_2: ::core::option::Option<
        ::ethereum_light_client_proto::ibc::lightclients::ethereum::v1::ConsensusUpdate,
    >,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct NextSyncCommitteeMisbehaviour {
    #[prost(string, tag = "1")]
    pub client_id: ::prost::alloc::string::String,
    #[prost(message, optional, tag = "2")]
    pub trusted_sync_committee: ::core::option::Option<
        ::ethereum_light_client_proto::ibc::lightclients::ethereum::v1::TrustedSyncCommittee,
    >,
    #[prost(message, optional, tag = "3")]
    pub consensus_update_1: ::core::option::Option<
        ::ethereum_light_client_proto::ibc::lightclients::ethereum::v1::ConsensusUpdate,
    >,
    #[prost(message, optional, tag = "4")]
    pub consensus_update_2: ::core::option::Option<
        ::ethereum_light_client_proto::ibc::lightclients::ethereum::v1::ConsensusUpdate,
    >,
}
