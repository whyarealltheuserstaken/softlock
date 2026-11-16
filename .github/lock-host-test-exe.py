#!/usr/bin/env python3
"""Print the test executable from a `cargo test --message-format=json --no-run` log."""

import json
import sys


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: lock-host-test-exe.py <cargo-json-log>", file=sys.stderr)
        return 1
    exe = ""
    with open(sys.argv[1], encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue
            found = msg.get("executable")
            if found:
                exe = found
    if not exe:
        print(f"test executable not found in {sys.argv[1]}", file=sys.stderr)
        return 1
    print(exe)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
