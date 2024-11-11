#!/bin/env bash

set -ex

LCP_BIN=${LCP_BIN:-./lcp/bin/lcp}
ENCLAVE_PATH=./bin/enclave.signed.so
CERTS_DIR=./lcp/tests/certs

export LCP_ENCLAVE_DEBUG=1

rm -rf ~/.lcp

enclave_key=$(${LCP_BIN} --log_level=off enclave generate-key --enclave=${ENCLAVE_PATH})

if [ -z "$SGX_MODE" ] || [ "$SGX_MODE" = "HW" ]; then
    ${LCP_BIN} attestation ias --enclave=${ENCLAVE_PATH} --enclave_key=${enclave_key}
else
    ${LCP_BIN} attestation simulate --enclave=${ENCLAVE_PATH} --enclave_key=${enclave_key} --signing_cert_path=${CERTS_DIR}/signing.crt.der --signing_key=${CERTS_DIR}/signing.key
fi

if [ "$SGX_MODE" = "SW" ]; then
    export LCP_RA_ROOT_CERT_HEX=$(cat ${CERTS_DIR}/root.crt | xxd -p -c 1000000)
fi

${LCP_BIN} --log_level=info service start --enclave=${ENCLAVE_PATH} --address=127.0.0.1:50051 --threads=2 &
