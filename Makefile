SCHEMA := schema/nmux.fbs
GEN_DIR := crates/nmux-proto/src/generated

.PHONY: check check-schema generate-schema rust-test

check: check-schema rust-test

check-schema:
	flatc --json --strict-json --no-warnings -o /tmp $(SCHEMA)

generate-schema:
	rm -rf $(GEN_DIR)
	mkdir -p $(GEN_DIR)
	flatc --rust -o $(GEN_DIR) $(SCHEMA)

rust-test:
	cargo test --workspace
