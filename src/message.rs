use crate::errors::Error;
use core::ops::Deref;
use ethereum_ibc::header::{Header as EthereumHeader, ETHEREUM_HEADER_TYPE_URL};
use ethereum_ibc::misbehaviour::{
    Misbehaviour as EthereumMisbehaviour, ETHEREUM_FINALIZED_HEADER_MISBEHAVIOUR_TYPE_URL,
    ETHEREUM_NEXT_SYNC_COMMITTEE_MISBEHAVIOUR_TYPE_URL,
};
use light_client::types::proto::google::protobuf::Any as IBCAny;
use light_client::types::Any;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum ClientMessage<const SYNC_COMMITTEE_SIZE: usize> {
    Header(Header<SYNC_COMMITTEE_SIZE>),
    Misbehaviour(Misbehaviour<SYNC_COMMITTEE_SIZE>),
}

impl<const SYNC_COMMITTEE_SIZE: usize> From<ClientMessage<SYNC_COMMITTEE_SIZE>> for Any {
    fn from(value: ClientMessage<SYNC_COMMITTEE_SIZE>) -> Self {
        match value {
            ClientMessage::Header(header) => header.into(),
            ClientMessage::Misbehaviour(misbehaviour) => misbehaviour.into(),
        }
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> TryFrom<Any> for ClientMessage<SYNC_COMMITTEE_SIZE> {
    type Error = Error;

    fn try_from(value: Any) -> Result<Self, Self::Error> {
        match value.type_url.as_str() {
            ETHEREUM_HEADER_TYPE_URL => Ok(ClientMessage::Header(Header::try_from(value)?)),
            ETHEREUM_FINALIZED_HEADER_MISBEHAVIOUR_TYPE_URL
            | ETHEREUM_NEXT_SYNC_COMMITTEE_MISBEHAVIOUR_TYPE_URL => {
                Ok(ClientMessage::Misbehaviour(Misbehaviour::try_from(value)?))
            }
            _ => Err(Error::UnexpectedClientType(value.type_url.clone())),
        }
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Misbehaviour<const SYNC_COMMITTEE_SIZE: usize>(
    pub(crate) EthereumMisbehaviour<SYNC_COMMITTEE_SIZE>,
);

impl<const SYNC_COMMITTEE_SIZE: usize> Deref for Misbehaviour<SYNC_COMMITTEE_SIZE> {
    type Target = EthereumMisbehaviour<SYNC_COMMITTEE_SIZE>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> From<Misbehaviour<SYNC_COMMITTEE_SIZE>>
    for EthereumMisbehaviour<SYNC_COMMITTEE_SIZE>
{
    fn from(value: Misbehaviour<SYNC_COMMITTEE_SIZE>) -> Self {
        value.0
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> From<EthereumMisbehaviour<SYNC_COMMITTEE_SIZE>>
    for Misbehaviour<SYNC_COMMITTEE_SIZE>
{
    fn from(value: EthereumMisbehaviour<SYNC_COMMITTEE_SIZE>) -> Self {
        Self(value)
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> From<Misbehaviour<SYNC_COMMITTEE_SIZE>> for Any {
    fn from(value: Misbehaviour<SYNC_COMMITTEE_SIZE>) -> Self {
        IBCAny::from(value.0).into()
    }
}

impl<const SYNC_COMMITTEE_SIZE: usize> TryFrom<Any> for Misbehaviour<SYNC_COMMITTEE_SIZE> {
    type Error = Error;

    fn try_from(value: Any) -> Result<Self, Self::Error> {
        let any: IBCAny = value.into();
        if any.type_url == ETHEREUM_FINALIZED_HEADER_MISBEHAVIOUR_TYPE_URL
            || any.type_url == ETHEREUM_NEXT_SYNC_COMMITTEE_MISBEHAVIOUR_TYPE_URL
        {
            Ok(Self(
                EthereumMisbehaviour::try_from(any).map_err(Error::ICS02)?,
            ))
        } else {
            Err(Error::UnexpectedClientType(any.type_url))
        }
    }
}
