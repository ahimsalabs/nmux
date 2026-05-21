.PHONY: check check-schema

check: check-schema

check-schema:
	flatc --json --strict-json --no-warnings -o /tmp schema/nmux.fbs
