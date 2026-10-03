use crate::errors::Error;
use crate::misbehaviour::{
    Misbehaviour, ETHEREUM_FINALIZED_HEADER_MISBEHAVIOUR_TYPE_URL,
    ETHEREUM_NEXT_SYNC_COMMITTEE_MISBEHAVIOUR_TYPE_URL,
};
use bytes::Buf;
use ethereum_consensus::types::U64;
use ethereum_elc_proto::ibc::lightclients::ethereum::v1::Header as RawHeader;
use ethereum_light_client_proto::google::protobuf::Any as IBCAny;
use ethereum_light_client_types::consensus::{
    convert_proto_to_consensus_update, convert_proto_to_execution_update, AccountUpdateInfo,
    ConsensusUpdateInfo, ExecutionUpdateInfo, TrustedSyncCommittee,
};
use ethereum_light_client_verifier::context::ChainConsensusVerificationContext;
use ethereum_light_client_verifier::updates::ConsensusUpdate;
use light_client::types::Time;
use prost::Message;

pub const ETHEREUM_HEADER_TYPE_URL: &str = "/ibc.lightclients.ethereum.v1.Header";

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum ClientMessage<const SYNC_COMMITTEE_SIZE: usize> {
    Header(Header<SYNC_COMMITTEE_SIZE>),
    Misbehaviour(Misbehaviour<SYNC_COMMITTEE_SIZE>),
}

impl<const SYNC_COMMITTEE_SIZE: usize> TryFrom<IBCAny> for ClientMessage<SYNC_COMMITTEE_SIZE> {
    type Error = Error;

    fn try_from(raw: IBCAny) -> Result<Self, Self::Error> {
        match raw.type_url.as_str() {
            ETHEREUM_HEADER_TYPE_URL => {
                let header = Header::<SYNC_COMMITTEE_SIZE>::try_from(raw)?;
                Ok(Self::Header(header))
            }
            ETHEREUM_FINALIZED_HEADER_MISBEHAVIOUR_TYPE_URL
            | ETHEREUM_NEXT_SYNC_COMMITTEE_MISBEHAVIOUR_TYPE_URL => {
                let misbehaviour = Misbehaviour::<SYNC_COMMITTEE_SIZE>::try_from(raw)?;
                Ok(Self::Misbehaviour(misbehaviour))
            }
            _ => Err(Error::UnknownMessageType(raw.type_url)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Header<const SYNC_COMMITTEE_SIZE: usize> {
    /// trusted sync committee corresponding to the period of the signature slot of the `consensus_update`
    pub trusted_sync_committee: TrustedSyncCommittee<SYNC_COMMITTEE_SIZE>,
    /// consensus update attested by the `trusted_sync_committee`
    pub consensus_update: ConsensusUpdateInfo<SYNC_COMMITTEE_SIZE>,
    /// execution update for the execution block the `consensus_update.finalized_header`
    /// references:
    /// - pre-Gloas: the `execution_payload` carried by the finalized beacon block, i.e. the
    ///   block produced at the finalized slot
    /// - Gloas: the block that `signed_execution_payload_bid.message.parent_block_hash` points
    ///   at. The payload produced at the finalized slot is revealed in a separate envelope and
    ///   is not provable from light client data, so this lags the finalized slot by one block.
    pub execution_update: ExecutionUpdateInfo,
    /// account update based on the `execution_update.state_root`
    pub account_update: AccountUpdateInfo,
}

pub fn decode_header<const SYNC_COMMITTEE_SIZE: usize, B: Buf>(
    buf: B,
) -> Result<Header<SYNC_COMMITTEE_SIZE>, Error> {
    RawHeader::decode(buf)
        .map_err(Error::ProtoDecode)?
        .try_into()
}

impl<const SYNC_COMMITTEE_SIZE: usize> Header<SYNC_COMMITTEE_SIZE> {
    pub fn validate(&self) -> Result<(), Error> {
        self.trusted_sync_committee.validate()?;
        if self.execution_update.block_number == U64(0) {
            return Err(Error::ZeroBlockNumber);
        }
        Ok(())
    }

    /// Timestamp of the execution block that `execution_update` describes:
    /// - pre-Gloas: `compute_timestamp_at_slot(finalized_slot)`
    /// - Gloas: the timestamp of the bid's parent block, which is neither the finalized slot's
    ///   timestamp nor derivable from it, since slots may be skipped
    ///
    /// Only call this once the execution update has been verified — the Gloas branch reads the
    /// RLP header, which is bound to the consensus update by that verification.
    pub fn timestamp<C: ChainConsensusVerificationContext>(&self, ctx: &C) -> Result<Time, Error> {
        let nanos = self
            .execution_update
            .timestamp(ctx, self.consensus_update.finalized_beacon_header().slot)?;
        Ok(Time::from_unix_timestamp_nanos(nanos)?)
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> TryFrom<RawHeader> for Header<SYNC_COMMITTEE_SIZE> {
    type Error = Error;
    fn try_from(value: RawHeader) -> Result<Self, Self::Error> {
        let trusted_sync_committee = value
            .trusted_sync_committee
            .ok_or(Error::proto_missing("trusted_sync_committee"))?;
        let consensus_update = value
            .consensus_update
            .ok_or(Error::proto_missing("consensus_update"))?;
        let execution_update = value
            .execution_update
            .ok_or(Error::proto_missing("execution_update"))?;
        let account_update = value
            .account_update
            .ok_or(Error::proto_missing("account_update"))?;
        Ok(Self {
            trusted_sync_committee: trusted_sync_committee.try_into()?,
            consensus_update: convert_proto_to_consensus_update(consensus_update)?,
            execution_update: convert_proto_to_execution_update(execution_update)?,
            account_update: account_update.try_into()?,
        })
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> TryFrom<IBCAny> for Header<SYNC_COMMITTEE_SIZE> {
    type Error = Error;

    fn try_from(raw: IBCAny) -> Result<Self, Self::Error> {
        use core::ops::Deref;

        match raw.type_url.as_str() {
            ETHEREUM_HEADER_TYPE_URL => decode_header(raw.value.deref()),
            _ => Err(Error::UnknownHeaderType {
                type_url: raw.type_url,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{to_consensus_update_info, TestFixture, SYNC_COMMITTEE_SIZE};
    use alloc::string::ToString;
    use ethereum_consensus::compute::compute_timestamp_at_slot;
    use ethereum_consensus::types::U64;
    use ethereum_light_client_types::consensus::{AccountUpdateInfo, ExecutionUpdateInfo};
    use ethereum_light_client_verifier::updates::ConsensusUpdate;
    use light_client::types::Height;

    #[test]
    fn test_client_message_from_any_unknown_type() {
        let any = IBCAny {
            type_url: "/unknown.type".to_string(),
            value: vec![],
        };
        let result = ClientMessage::<SYNC_COMMITTEE_SIZE>::try_from(any);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            Error::UnknownMessageType(ref url) if url == "/unknown.type"
        ));
    }

    #[test]
    fn test_header_from_any_unknown_type() {
        let any = IBCAny {
            type_url: "/unknown.header".to_string(),
            value: vec![],
        };
        let result = Header::<SYNC_COMMITTEE_SIZE>::try_from(any);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            Error::UnknownHeaderType { type_url } if type_url == "/unknown.header"
        ));
    }

    #[test]
    fn test_header_validate_success() {
        let fixture = TestFixture::simple(1_000_000);
        let (update, _) = fixture.gen_update([1u8; 32].into(), 100);

        let header = Header {
            trusted_sync_committee: fixture.trusted_from_current(Height::new(0, 1), true),
            consensus_update: to_consensus_update_info(update),
            execution_update: ExecutionUpdateInfo {
                block_number: U64(100),
                ..Default::default()
            },
            account_update: AccountUpdateInfo::default(),
        };

        let result = header.validate();
        assert!(result.is_ok(), "header validation failed: {:?}", result);
    }

    #[test]
    fn test_header_validate_zero_block_number() {
        let fixture = TestFixture::simple(1_000_000);
        let (update, _) = fixture.gen_update([1u8; 32].into(), 100);

        let update_info = to_consensus_update_info(update);
        let header = Header {
            trusted_sync_committee: fixture.trusted_from_current(Height::new(0, 1), true),
            consensus_update: update_info,
            execution_update: ExecutionUpdateInfo {
                block_number: U64(0), // Zero block number
                ..Default::default()
            },
            account_update: AccountUpdateInfo::default(),
        };

        let result = header.validate();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::ZeroBlockNumber));
    }

    #[test]
    fn test_header_timestamp_is_derived_from_the_finalized_slot_pre_gloas() {
        let fixture = TestFixture::simple(1_000_000);
        let (update, _) = fixture.gen_update([1u8; 32].into(), 100);

        let update_info = to_consensus_update_info(update);
        let finalized_slot = update_info.finalized_beacon_header().slot;

        let header = Header {
            trusted_sync_committee: fixture.trusted_from_current(Height::new(0, 1), true),
            consensus_update: update_info,
            execution_update: ExecutionUpdateInfo {
                block_number: U64(100),
                ..Default::default()
            },
            account_update: AccountUpdateInfo::default(),
        };

        // no timestamp is supplied by the relayer; it comes from the verified finalized slot
        let expected = compute_timestamp_at_slot(&fixture.ctx, finalized_slot).0;
        let timestamp = header.timestamp(&fixture.ctx).unwrap();
        assert_eq!(timestamp.as_unix_timestamp_secs(), expected);
    }
}
