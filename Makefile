.PHONY: check fmt lint test gate

check:
	cargo qk

fmt:
	cargo qa

lint:
	cargo qc

test:
	cargo qt

gate: fmt lint test
