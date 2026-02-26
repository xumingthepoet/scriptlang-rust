.PHONY: check docs fmt lint test coverage gate gate-verbose

check:
	cargo qk

fmt:
	cargo qa

lint:
	cargo qc

test:
	env -u LLVM_PROFILE_FILE cargo qt

coverage:
	cargo llvm-cov --workspace --exclude sl-cli --all-features --all-targets --summary-only --fail-under-lines 100

gate:
	$(MAKE) check
	$(MAKE) fmt
	$(MAKE) lint
	$(MAKE) test
	$(MAKE) coverage
