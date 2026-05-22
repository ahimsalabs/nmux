SCHEMA := schema/nmux.fbs
GEN_DIR := crates/nmux-proto/src/generated

.PHONY: check check-ghostty-vt check-schema generate-schema rust-test

check: check-schema rust-test

check-ghostty-vt:
	GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-core --features libghostty-vt ghostty_vt
	GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-cli --features libghostty-vt live_cli_can_use_libghostty_vt_terminal_engine

check-schema:
	flatc --json --strict-json --no-warnings -o /tmp $(SCHEMA)

generate-schema:
	rm -rf $(GEN_DIR)
	mkdir -p $(GEN_DIR)
	flatc --rust -o $(GEN_DIR) $(SCHEMA)

rust-test:
	cargo test --workspace
