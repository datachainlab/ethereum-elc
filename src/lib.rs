#![cfg_attr(not(test), no_std)]
#![allow(clippy::result_large_err)]
#![allow(clippy::large_enum_variant)]

use client::EthereumLightClient;
use ethereum_ibc::client_state::ETHEREUM_CLIENT_STATE_TYPE_URL;
use light_client::LightClientRegistry;
extern crate alloc;

pub mod client;
pub mod errors;
pub mod state;
pub use ethereum_ibc as ibc;

#[allow(unused_imports)]
mod internal_prelude {
    pub use alloc::boxed::Box;
    pub use alloc::format;
    pub use alloc::string::{String, ToString};
    pub use alloc::vec;
    pub use alloc::vec::Vec;
}
use internal_prelude::*;

pub fn register_implementations<const SYNC_COMMITTEE_SIZE: usize>(
    registry: &mut dyn LightClientRegistry,
) {
    registry
        .put_light_client(
            ETHEREUM_CLIENT_STATE_TYPE_URL.to_string(),
            Box::new(EthereumLightClient::<SYNC_COMMITTEE_SIZE>),
        )
        .unwrap()
}
