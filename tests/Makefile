include enclave.mk

.PHONY: all
all: $(Signed_RustEnclave_Name) lcp yrly

.PHONY: lcp
lcp:
	$(MAKE) -C ./lcp all

.PHONY: yrly
yrly:
	go build -o ./bin/yrly -tags customcert ./relayer/main.go

.PHONY: e2e-test
setup:
	./scripts/run_lcp.sh
	./scripts/run_rly.sh
