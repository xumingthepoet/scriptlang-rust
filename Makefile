.PHONY: check docs fmt lint test coverage gate gate-verbose

check:
	cargo check --workspace --all-targets --all-features

fmt:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
	env -u LLVM_PROFILE_FILE cargo test --workspace --all-targets --all-features 

coverage:
	bash scripts/coverage.sh

gate:
	$(MAKE) check
	$(MAKE) fmt
	$(MAKE) lint
	$(MAKE) test
	$(MAKE) coverage
