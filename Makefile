.PHONY: check docs fmt lint test coverage gate gate-verbose

check:
	cargo qk

fmt:
	cargo qa

lint:
	cargo qc

test:
	cargo qt

coverage:
	bash scripts/gate.sh coverage

gate:
	bash scripts/gate.sh gate

gate-verbose:
	$(MAKE) check
	$(MAKE) fmt
	$(MAKE) lint
	$(MAKE) test
	cargo tarpaulin --engine llvm --workspace --all-features --all-targets --rustflags=--cfg=coverage --out Stdout --fail-under 100
