use crate::errors::Error;
use core::ops::Deref;
use ethereum_ibc::header::{Header as EthereumHeader, ETHEREUM_HEADER_TYPE_URL};
use light_client::types::proto::google::protobuf::Any as IBCAny;
use light_client::types::Any;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Header<const SYNC_COMMITTEE_SIZE: usize>(pub(crate) EthereumHeader<SYNC_COMMITTEE_SIZE>);

impl<const SYNC_COMMITTEE_SIZE: usize> Deref for Header<SYNC_COMMITTEE_SIZE> {
    type Target = EthereumHeader<SYNC_COMMITTEE_SIZE>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> From<Header<SYNC_COMMITTEE_SIZE>>
    for EthereumHeader<SYNC_COMMITTEE_SIZE>
{
    fn from(value: Header<SYNC_COMMITTEE_SIZE>) -> Self {
        value.0
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> From<EthereumHeader<SYNC_COMMITTEE_SIZE>>
    for Header<SYNC_COMMITTEE_SIZE>
{
    fn from(value: EthereumHeader<SYNC_COMMITTEE_SIZE>) -> Self {
        Self(value)
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> From<Header<SYNC_COMMITTEE_SIZE>> for Any {
    fn from(value: Header<SYNC_COMMITTEE_SIZE>) -> Self {
        IBCAny::from(value.0).into()
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> TryFrom<Any> for Header<SYNC_COMMITTEE_SIZE> {
    type Error = Error;

    fn try_from(value: Any) -> Result<Self, Self::Error> {
        let any: IBCAny = value.into();
        if any.type_url == ETHEREUM_HEADER_TYPE_URL {
            Ok(Self(EthereumHeader::try_from(any).map_err(Error::ICS02)?))
        } else {
            Err(Error::UnexpectedClientType(any.type_url))
        }
    }
}
