# Create .env file if it doesn't exist
$(shell [ ! -f .env ] && touch .env)

# Load environment variables from .env file
include .env

# Set default env variables if not set in .env
export RUST_BACKTRACE ?= full
export RUST_LOG ?= info,nockchain=info,nockchain_libp2p_io=info,libp2p=info,libp2p_quic=info
export MINIMAL_LOG_FORMAT ?= true
export MINING_PKH ?= 9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV
export

DOCKER_IMAGE ?= nockchain-local
DOCKER_MEM ?= 32g
# DOCKER_MEM ?= 16g
# DOCKER_MEM_SWAP ?= 32g
DOCKER_P2P_PORT ?= 30000
DOCKER_DATA_DIR ?= $(CURDIR)/.data.nockchain
DOCKER_NOCKCHAIN_ENVS ?=
DOCKER_NOCKCHAIN_ARGS ?=
DOCKER_METRICS_COMPOSE ?= docker-compose.metrics.yml
DOCKER_METRICS_NETWORK ?= nockchain-metrics
INFLUXDB_VERSION ?= 2.7
TELEGRAF_VERSION ?= 1.30
STATSD_HOST ?= telegraf
STATSD_PORT ?= 8125

.PHONY: build
build: build-hoon-all build-rust
	$(call show_env_vars)

## Build all rust
.PHONY: build-rust
build-rust:
	cargo build --release

.PHONY: build-nockchain-jemalloc
build-nockchain-jemalloc:
	cargo build --release --features jemalloc --bin nockchain

## Run all tests
.PHONY: test
test:
	cargo test --release

.PHONY: bench-nockchain-kernel
bench-nockchain-kernel:
	cargo run --release -p nockchain --bin bench_nockchain_kernel -- --skip-mining

.PHONY: test-pma-paging-kernel
test-pma-paging-kernel:
	NOCKCHAIN_PMA_PAGING_SKIP_MINING=1 NOCKCHAIN_PMA_PAGING_SKIP_TXS=1 NOCKCHAIN_PMA_PAGING_BLOCKS=250 NOCKCHAIN_PMA_PAGING_BYTES=1073741824 NOCKCHAIN_PMA_PAGING_OUTPUTS=1 cargo test --release --test pma_paging_kernel -- --ignored
# NOCKCHAIN_PMA_PAGING_SKIP_MINING=1 NOCKCHAIN_PMA_PAGING_BLOCKS=100000 NOCKCHAIN_PMA_PAGING_BYTES=1073741824 NOCKCHAIN_PMA_PAGING_OUTPUTS=128 cargo test --release --test pma_paging_kernel -- --ignored

.PHONY: test-pma-persist-blocks
test-pma-persist-blocks:
	cargo test --release --test pma_persist_blocks

.PHONY: docker-nockchain
docker-nockchain: docker-nockchain-build docker-nockchain-run

.PHONY: docker-nockchain-pma-persist
docker-nockchain-pma-persist: docker-nockchain-build
	$(MAKE) docker-nockchain-run \
		DOCKER_NOCKCHAIN_ENVS="-e NOCK_PMA_PERSIST=1" \
		DOCKER_NOCKCHAIN_ARGS="--pma-persist $(DOCKER_NOCKCHAIN_ARGS)"

.PHONY: docker-nockchain-build
docker-nockchain-build:
	docker build -t $(DOCKER_IMAGE) .

.PHONY: docker-nockchain-run
docker-nockchain-run:
	mkdir -p $(DOCKER_DATA_DIR)
	@docker network inspect $(DOCKER_METRICS_NETWORK) >/dev/null 2>&1 || docker network create $(DOCKER_METRICS_NETWORK)
	docker run --rm -it --name nockchain \
		--network $(DOCKER_METRICS_NETWORK) \
		--memory $(DOCKER_MEM) \
		-e RUST_BACKTRACE=1 \
		-e RUST_LOG=info,nockapp::nockapp::save=trace \
		-e NOCK_PMA_TIMING=1 \
		-e NOCK_PMA_TIMING_DETAIL=1 \
		-e NOCK_STACK_TIMING_DETAIL=1 \
		-e STATSD_HOST=$(STATSD_HOST) \
		-e STATSD_PORT=$(STATSD_PORT) \
		$(DOCKER_NOCKCHAIN_ENVS) \
		-p $(DOCKER_P2P_PORT):$(DOCKER_P2P_PORT)/udp \
		-v $(DOCKER_DATA_DIR):/data/.data.nockchain \
		$(DOCKER_IMAGE) \
		--fast-sync --num-threads 0 \
		--save-interval 300000 \
		--data-dir /data/.data.nockchain \
		--identity-path /data/.data.nockchain/.nockchain_identity \
		--bind /ip4/0.0.0.0/udp/$(DOCKER_P2P_PORT)/quic-v1 \
		$(DOCKER_NOCKCHAIN_ARGS)

.PHONY: docker-metrics
docker-metrics:
	@docker network inspect $(DOCKER_METRICS_NETWORK) >/dev/null 2>&1 || docker network create $(DOCKER_METRICS_NETWORK)
	docker compose -f $(DOCKER_METRICS_COMPOSE) up -d

.PHONY: fmt
fmt:
	cargo fmt

.PHONY: build-hoonc
build-hoonc: nuke-hoonc-data ## Build hoonc from this repo
	$(call show_env_vars)
	cargo build --release --locked --bin hoonc

.PHONY: build-hoonc-tracing
build-hoonc-tracing: nuke-hoonc-data ## Build hoonc with tracing
	$(call show_env_vars)
	cargo build --release --bin hoonc --features tracing-tracy

.PHONY: install-hoonc
install-hoonc: nuke-hoonc-data ## Install hoonc from this repo
	$(call show_env_vars)
	cargo install --locked --force --path crates/hoonc --bin hoonc

.PHONY: update-hoonc
update-hoonc:
	$(call show_env_vars)
	cargo install --locked --path crates/hoonc --bin hoonc

.PHONY: build-nockchain
build-nockchain: assets/dumb.jam assets/miner.jam
	$(call show_env_vars)
	cargo build --release --bin nockchain --features tracing-tracy

.PHONY: install-nockchain
install-nockchain: assets/dumb.jam assets/miner.jam
	$(call show_env_vars)
	cargo install --locked --force --path crates/nockchain --bin nockchain

.PHONY: install-nockchain-wallet
install-nockchain-wallet: assets/wal.jam
	$(call show_env_vars)
	cargo install --locked --force --path crates/nockchain-wallet --bin nockchain-wallet

.PHONY: install-nockchain-peek
install-nockchain-peek: assets/peek.jam
	$(call show_env_vars)
	cargo install --locked --force --path crates/nockchain-peek --bin nockchain-peek

.PHONY: ensure-dirs
ensure-dirs:
	mkdir -p hoon
	mkdir -p assets

.PHONY: build-trivial
build-trivial: ensure-dirs
	$(call show_env_vars)
	echo '%trivial' > hoon/trivial.hoon
	hoonc --arbitrary hoon/trivial.hoon

HOON_TARGETS=assets/dumb.jam assets/wal.jam assets/miner.jam assets/peek.jam

.PHONY: nuke-hoonc-data
nuke-hoonc-data:
	rm -rf .data.hoonc
	rm -rf ~/.nockapp/hoonc

.PHONY: nuke-assets
nuke-assets:
	rm -f assets/*.jam

.PHONY: build-hoon-all
build-hoon-all: nuke-assets update-hoonc ensure-dirs build-trivial $(HOON_TARGETS)
	$(call show_env_vars)

.PHONY: build-hoon
build-hoon: ensure-dirs update-hoonc $(HOON_TARGETS)
	$(call show_env_vars)

.PHONY: build-assets
build-assets: ensure-dirs $(HOON_TARGETS)
	$(call show_env_vars)

HOON_SRCS := $(find hoon -type file -name '*.hoon')

## Build dumb.jam with hoonc
assets/dumb.jam: ensure-dirs hoon/apps/dumbnet/outer.hoon $(HOON_SRCS)
	$(call show_env_vars)
	rm -f assets/dumb.jam
	hoonc hoon/apps/dumbnet/outer.hoon hoon
	mv out.jam assets/dumb.jam

## Build wal.jam with hoonc
assets/wal.jam: ensure-dirs hoon/apps/wallet/wallet.hoon $(HOON_SRCS)
	$(call show_env_vars)
	rm -f assets/wal.jam
	hoonc hoon/apps/wallet/wallet.hoon hoon
	mv out.jam assets/wal.jam

## Build mining.jam with hoonc
assets/miner.jam: ensure-dirs hoon/apps/dumbnet/miner.hoon $(HOON_SRCS)
	$(call show_env_vars)
	rm -f assets/miner.jam
	hoonc hoon/apps/dumbnet/miner.hoon hoon
	mv out.jam assets/miner.jam

## Build peek.jam with hoonc
assets/peek.jam: ensure-dirs hoon/apps/peek/peek.hoon $(HOON_SRCS)
	$(call show_env_vars)
	rm -f assets/peek.jam
	hoonc hoon/apps/peek/peek.hoon hoon
	mv out.jam assets/peek.jam
