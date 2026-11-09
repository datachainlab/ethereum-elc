######## SGX SDK Settings ########
SGX_SDK ?= /opt/sgxsdk
SGX_MODE ?= HW
SGX_ARCH ?= x64
SGX_DEBUG ?= 0
SGX_PRERELEASE ?= 0
SGX_PRODUCTION ?= 0

include buildenv.mk

ifeq ($(shell getconf LONG_BIT), 32)
	SGX_ARCH := x86
else ifeq ($(findstring -m32, $(CXXFLAGS)), -m32)
	SGX_ARCH := x86
endif

ifeq ($(SGX_ARCH), x86)
	SGX_COMMON_CFLAGS := -m32
	SGX_LIBRARY_PATH := $(SGX_SDK)/lib
	SGX_ENCLAVE_SIGNER := $(SGX_SDK)/bin/x86/sgx_sign
	SGX_EDGER8R := $(SGX_SDK)/bin/x86/sgx_edger8r
else
	SGX_COMMON_CFLAGS := -m64
	SGX_LIBRARY_PATH := $(SGX_SDK)/lib64
	SGX_ENCLAVE_SIGNER := $(SGX_SDK)/bin/x64/sgx_sign
	SGX_EDGER8R := $(SGX_SDK)/bin/x64/sgx_edger8r
endif

ifeq ($(SGX_DEBUG), 1)
ifeq ($(SGX_PRERELEASE), 1)
$(error Cannot set SGX_DEBUG and SGX_PRERELEASE at the same time!!)
endif
ifeq ($(SGX_PRODUCTION), 1)
$(error Cannot set SGX_DEBUG and SGX_PRODUCTION at the same time!!)
endif
endif

ifeq ($(SGX_DEBUG), 1)
	# we build with cargo --release, even in SGX DEBUG mode
	SGX_COMMON_CFLAGS += -O0 -g -ggdb
	# cargo sets this automatically, cannot use 'debug'
	OUTPUT_PATH := release
	CARGO_TARGET := --release
else
	SGX_COMMON_CFLAGS += -O2
	OUTPUT_PATH := release
	CARGO_TARGET := --release
endif

SGX_COMMON_CFLAGS += -fstack-protector

ENCLAVE_CARGO_FEATURES = --features=default
APP_CARGO_FEATURES     = --features=default
ifeq ($(SGX_PRODUCTION), 1)
	SGX_ENCLAVE_MODE = "Production Mode"
	SGX_ENCLAVE_CONFIG = $(SGX_ENCLAVE_CONFIG)
	SGX_SIGN_KEY = $(SGX_COMMERCIAL_KEY)
else
	SGX_ENCLAVE_MODE = "Development Mode"
	SGX_ENCLAVE_CONFIG = "enclave/Enclave.config.xml"
	SGX_SIGN_KEY = "enclave/Enclave_private.pem"
	ifneq ($(SGX_MODE), HW)
		ENCLAVE_CARGO_FEATURES = --features=default
		APP_CARGO_FEATURES     = --features=default,sgx-sw
	endif
endif

######## CUSTOM Settings ########

CUSTOM_LIBRARY_PATH := ./lib
CUSTOM_BIN_PATH := ./bin

######## Enclave Settings ########

ifneq ($(SGX_MODE), HW)
	Trts_Library_Name := sgx_trts_sim
	Service_Library_Name := sgx_tservice_sim
else
	Trts_Library_Name := sgx_trts
	Service_Library_Name := sgx_tservice
endif
Crypto_Library_Name := sgx_tcrypto
ProtectedFs_Library_Name := sgx_tprotected_fs

# avx2 and the link flags that used to live here are in the target spec now. The
# spec comes from the lcp submodule, so this enclave is built against exactly the
# one lcp's own enclave uses.
Enclave_Target_Spec := $(CURDIR)/lcp/enclave/x86_64-unknown-unknown-sgx.json

RustEnclave_Name := enclave/target/x86_64-unknown-unknown-sgx/$(OUTPUT_PATH)/enclave.so
Signed_RustEnclave_Name := bin/enclave.signed.so

######## Enclave Objects ########

$(Signed_RustEnclave_Name): enclave
	mkdir -p bin
	@$(SGX_ENCLAVE_SIGNER) sign -key enclave/Enclave_private.pem -enclave $(RustEnclave_Name) -out $@ -config enclave/Enclave.config.xml
	@echo "SIGN =>  $@"

# cargo sgx build runs edger8r for the trusted half through enclave/build.rs, so
# there is no EDL rule here. build-std goes through the environment, so the
# enclave needs no .cargo/config.toml of its own.
.PHONY: enclave
enclave:
	@cd enclave && SGX_SDK=$(SGX_SDK) SGX_MODE=$(SGX_MODE) \
		CARGO_UNSTABLE_BUILD_STD=core,alloc \
		cargo sgx build --target $(Enclave_Target_Spec) $(CARGO_TARGET)
