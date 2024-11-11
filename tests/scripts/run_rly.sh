#!/bin/sh
set -eux
LCP_BIN=${LCP_BIN:-./lcp/bin/lcp}
RLY=${RLY:-./bin/yrly}
TEMPLATE_DIR=./configs/templates
CONFIG_DIR=./configs/demo
CHAINID_ONE=ibc0
CHAINID_TWO=ibc1
PATH_NAME=ibc01

EXECUTION_ENDPOINT=${EXECUTION_ENDPOINT:-http://localhost:8545}
CONSENSUS_ENDPOINT=${CONSENSUS_ENDPOINT:-http://localhost:9596}

rm -rf ~/.yui-relayer

mkdir -p $CONFIG_DIR
MRENCLAVE=$(${LCP_BIN} enclave metadata --enclave=./bin/enclave.signed.so | jq -r .mrenclave)
jq -n -f ${TEMPLATE_DIR}/ibc-0.json.tpl --arg MRENCLAVE ${MRENCLAVE} > ${CONFIG_DIR}/ibc-0.json
jq -n -f ${TEMPLATE_DIR}/ibc-1.json.tpl --arg MRENCLAVE ${MRENCLAVE} \
    --arg IBC_ADDRESS 0xFF00000000000000000000000000000000000000 \
    --arg EXECUTION_ENDPOINT ${EXECUTION_ENDPOINT} \
    --arg CONSENSUS_ENDPOINT ${CONSENSUS_ENDPOINT} > ${CONFIG_DIR}/ibc-1.json

${RLY} config init
${RLY} chains add-dir configs/demo/
${RLY} paths add $CHAINID_ONE $CHAINID_TWO $PATH_NAME --file=./configs/path.json
