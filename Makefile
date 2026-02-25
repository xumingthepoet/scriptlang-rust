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
	cargo tarpaulin --engine llvm --workspace --all-features --all-targets --rustflags=--cfg=coverage --out Stdout --fail-under 100

gate: check fmt lint test coverage
