SCHEMA := schema/nmux.fbs
GEN_DIR := internal/protocol/flat

.PHONY: check check-schema generate-schema go-test

check: check-schema go-test

check-schema:
	flatc --json --strict-json --no-warnings -o /tmp $(SCHEMA)

generate-schema:
	rm -rf $(GEN_DIR)/protocol
	mkdir -p $(GEN_DIR)
	flatc --go --go-namespace protocol -o $(GEN_DIR) $(SCHEMA)

go-test:
	go test ./...
