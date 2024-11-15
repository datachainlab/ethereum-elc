# Integration tests

## Overview

This provides an integration test environment for yui-relayer and lcp including ethereum-elc. Currently, it is possible to test create-client and update-client.

## Pre-requisites

### Setup for Ethereum

You must set the below environment variables:

```bash
export EXECUTION_ENDPOINT=http://localhost:8545 // Must modify with your endpoint
export CONSENSUS_ENDPOINT=http://localhost:9596 // Must modify with your endpoint
```

If you cannot access the ethereum endpoint, you can launch a local ethereum node with the following command:

```bash
$ git clone https://github.com/datachainlab/cosmos-ethereum-ibc-lcp
$ make prepare-contracts build-images
$ make -C ./tests/e2e/chains/ethereum network
# if you want to stop the network, run `make -C ./tests/e2e/chains/ethereum network-down`
```

Then, you can set the environment variables as the following:

```bash
export EXECUTION_ENDPOINT=http://localhost:8546
export CONSENSUS_ENDPOINT=http://localhost:19596
```

### Modify the public testnet

If you want to run the tests with the public testnet like sepolia, you need to modify `enclave/lib.rs` as the following:

```rust
fn build_lc_registry() -> MapLightClientRegistry {
    let mut registry = MapLightClientRegistry::new();
    tendermint_lc::register_implementations(&mut registry);
    ethereum_elc::register_implementations::<{ ethereum_elc::ibc::consensus::preset::mainnet::PRESET.SYNC_COMMITTEE_SIZE }>(&mut registry);
    registry.seal().unwrap();
    registry
}
```

Also, you need to modify `./config/templates/ibc-1.json.tpl` as the following:

```
  "prover": {
    "@type": "/relayer.provers.lcp.config.ProverConfig",
    "origin_prover": {
      "@type": "/relayer.provers.ethereum_light_client.config.ProverConfig",
      "beacon_endpoint": $CONSENSUS_ENDPOINT,
      "network": "sepolia", // EDIT: Change to sepolia
      "trusting_period": "168h",
      "max_clock_drift": "0",
      "refresh_threshold_rate": {
        "numerator": 2,
        "denominator": 3
      }
    },
   ...
  }
```

### Install SGX-SDK

Please refer to the [cosmos-ethereum-ibc-lcp's README](https://github.com/datachainlab/cosmos-ethereum-ibc-lcp?tab=readme-ov-file#prerequisites).

## Build

```bash
$ make
```

## Run tests

```bash
# initialize the relayer and run lcp service
$ make setup
# set the test certificate for the relayer
$ export LCP_RA_ROOT_CERT_HEX=$(cat ./lcp/tests/certs/root.crt | xxd -p -c 1000000)
# ensure we can get the finality update from lodestar. If you got an error, please retry after a few seconds.
$ curl ${CONSENSUS_ENDPOINT}/eth/v1/beacon/light_client/finality_update
# run the tests
$ ./bin/yrly lcp create-elc ibc01 --src=false --elc_client_id=ethereum-0
$ ./bin/yrly lcp update-elc ibc01 --src=false --elc_client_id=ethereum-0
```

## Restart the lcp service

Please use `kill` or `pkill` to kill lcp service and run `Run tests` again.
