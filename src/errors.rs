use crate::internal_prelude::*;
use displaydoc::Display;
use light_client::LightClientSpecificError;

#[derive(Debug, Display)]
pub enum Error {
    /// ethereum ibc error: `{0}`
    IBC(ethereum_ibc::errors::Error),
    /// lcp commitments error: `{0}`
    Commitments(commitments::Error),
    /// ics02 error: `{0}`
    ICS02(ibc::core::ics02_client::error::ClientError),
    /// ics23 error: `{0}`
    ICS23(ibc::core::ics23_commitment::error::CommitmentError),
    /// ics24 error: `{0}`
    ICS24Path(ibc::core::ics24_host::path::PathError),
    /// unexpected client type: `{0}`
    UnexpectedClientType(String),
}

impl LightClientSpecificError for Error {}

impl From<commitments::Error> for Error {
    fn from(value: commitments::Error) -> Self {
        Self::Commitments(value)
    }
}
