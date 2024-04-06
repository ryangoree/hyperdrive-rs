.PHONY: build build-sol build-rust test test-sol test-rust lint lint-rust \
	lint-sol code-size-check solhint style-check spell-check warnings-check \
	prettier

### Build ###

build: 
	make build-sol && make build-rust

build-sol: 
	forge build

build-rust: 
	cargo build

### Test ###

SOLIDITY_LP_WITHDRAWAL_TESTS = LPWithdrawalTest
SOLIDITY_NETTING_TESTS = IntraCheckpointNettingTest
SOLIDITY_ZOMBIE_TESTS = ZombieInterestTest

test: 
	make test-sol && make test-rust

test-instances:
	forge test -vv --match-path test/instances/*.t.sol

test-sol-core:
	forge test -vv --no-match-contract "$(SOLIDITY_LP_WITHDRAWAL_TESTS)|$(SOLIDITY_NETTING_TESTS)|$(SOLIDITY_ZOMBIE_TESTS)"

# NOTE: Breaking these out onto a separate machine speeds up CI execution.
test-sol-lp-withdrawal:
	forge test -vv --match-contract "$(SOLIDITY_LP_WITHDRAWAL_TESTS)"

# NOTE: Breaking these out onto a separate machine speeds up CI execution.
test-sol-netting:
	forge test -vv --match-contract "$(SOLIDITY_NETTING_TESTS)"

# NOTE: Breaking these out onto a separate machine speeds up CI execution.
test-sol-zombie:
	forge test -vv --match-contract "$(SOLIDITY_ZOMBIE_TESTS)"

test-rust:
	cargo test --workspace --exclude hyperdrive-math && \
	cargo test --package hyperdrive-math -- --test-threads=1

### Lint ###

lint:
	make lint-sol && make lint-rust

lint-sol:
	make solhint && make style-check && make spell-check && make warnings-check && make code-size-check

code-size-check:
	FOUNDRY_PROFILE=production forge build && python3 python/contract_size.py out

solhint:
	npx solhint -f table 'contracts/src/**/*.sol'

spell-check:
	npx cspell ./**/**/**.sol --gitignore

style-check:
	npx prettier --check .

warnings-check:
	FOUNDRY_PROFILE=production forge build --deny-warnings --force

lint-rust:
	cargo check && cargo clippy && cargo fmt --check

### Prettier ###

prettier:
	npx prettier --write .
