#!/usr/bin/env python3
"""Print one test executable from a `cargo test --message-format=json --no-run` log.

Cargo also emits dependency binaries such as softlock-lockhost.exe. Pass a
needle, for example service_session, so the seam 3 job runs that test binary.
"""

import json
import sys


def main() -> int:
    if len(sys.argv) not in (2, 3):
        print(
            "usage: lock-host-test-exe.py <cargo-json-log> [needle]",
            file=sys.stderr,
        )
        return 1
    needle = sys.argv[2] if len(sys.argv) == 3 else ""
    found = []
    with open(sys.argv[1], encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue
            executable = msg.get("executable")
            if executable:
                found.append(executable)
    if needle:
        matched = [item for item in found if needle in item.replace("\\", "/")]
        if len(matched) != 1:
            print(
                f"expected one executable containing {needle}, found {matched or found}",
                file=sys.stderr,
            )
            return 1
        print(matched[0])
        return 0
    if not found:
        print(f"test executable not found in {sys.argv[1]}", file=sys.stderr)
        return 1
    print(found[-1])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
