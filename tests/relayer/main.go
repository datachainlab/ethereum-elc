package main

import (
	"fmt"
	"os"

	"github.com/datachainlab/ethereum-ibc-relay-chain/pkg/relay/ethereum"
	ethereumlc "github.com/datachainlab/ethereum-ibc-relay-prover/relay"
	"github.com/datachainlab/ibc-hd-signer/pkg/hd"
	lcp "github.com/datachainlab/lcp-go/relay"
	tendermint "github.com/hyperledger-labs/yui-relayer/chains/tendermint/module"
	"github.com/hyperledger-labs/yui-relayer/cmd"
)

func main() {
	if err := cmd.Execute(
		ethereum.Module{},
		ethereumlc.Module{},
		hd.Module{},
		lcp.Module{},
		tendermint.Module{},
	); err != nil {
		fmt.Fprintln(os.Stderr, "Error:", err)
		os.Exit(1)
	}
}
