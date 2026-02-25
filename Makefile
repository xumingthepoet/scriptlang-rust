.PHONY: check docs fmt lint test coverage gate

check:
	cargo qk

fmt:
	cargo qa

lint:
	cargo qc

test:
	cargo qt

coverage:
	cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 100 --show-missing-lines 

gate: check fmt lint test coverage
