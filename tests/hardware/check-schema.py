#!/usr/bin/env python3
"""Conformance check for softlock-hardware-suite/1 result.json."""

import json
import sys

SCHEMA = "softlock-hardware-suite/1"
SUITES = {"linux", "macos", "windows"}
STATUSES = {"pass", "fail", "skip"}
SKIP_REASONS = {
    "no-input",
    "budget",
    "needs-admin",
    "not-applicable",
    "ci",
    "operator",
    "interrupted",
}
STATES = {"never-unlocked", "os-locked"}
REQUIRED = {
    "schema",
    "suite",
    "started_utc",
    "finished_utc",
    "duration_ms",
    "mode",
    "config",
    "tests",
    "summary",
    "fail_closed",
    "serial_collected",
    "restored",
    "interrupted",
    "state",
}
ROW_FIELDS = {
    "id",
    "name",
    "status",
    "skip_reason",
    "physical",
    "needs_admin",
    "duration_ms",
    "detail",
    "measurements",
}
BANNED = (
    "product_serial",
    "board_serial",
    "chassis_serial",
    "Serial Number",
    "IOPlatformSerialNumber",
    "EVIOCGUNIQ",
)


def fail(message: str) -> None:
    print(f"schema: {message}", file=sys.stderr)
    raise SystemExit(1)


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: check-schema.py result.json")
    path = sys.argv[1]
    text = open(path, encoding="utf-8").read()
    for needle in BANNED:
        if needle in text:
            fail(f"banned marker {needle}")
    try:
        report = json.loads(text)
    except json.JSONDecodeError as err:
        fail(f"not json: {err}")
    if not isinstance(report, dict):
        fail("root must be an object")
    missing = REQUIRED - report.keys()
    if missing:
        fail(f"missing {sorted(missing)}")
    if report["schema"] != SCHEMA:
        fail(f"schema {report['schema']!r}")
    if report["suite"] not in SUITES:
        fail(f"suite {report['suite']!r}")
    if report["serial_collected"] is not False:
        fail("serial_collected must be false")
    if report["state"] not in STATES:
        fail(f"state {report['state']!r}")
    if not isinstance(report["config"], dict):
        fail("config must be an object")
    if not isinstance(report["duration_ms"], int):
        fail("duration_ms must be an int")
    tests = report["tests"]
    if not isinstance(tests, list) or not tests:
        fail("tests must be a non-empty array")
    counts = {"pass": 0, "fail": 0, "skip": 0}
    for row in tests:
        if not isinstance(row, dict):
            fail("test row must be an object")
        if set(row) != ROW_FIELDS and not ROW_FIELDS <= set(row):
            fail(f"row {row.get('id')} missing fields")
        status = row["status"]
        if status not in STATUSES:
            fail(f"{row.get('id')} status {status!r}")
        counts[status] += 1
        reason = row["skip_reason"]
        if status == "skip":
            if reason not in SKIP_REASONS:
                fail(f"{row.get('id')} skip_reason {reason!r}")
        elif reason is not None:
            fail(f"{row.get('id')} skip_reason set on {status}")
        if not isinstance(row["physical"], bool) or not isinstance(row["needs_admin"], bool):
            fail(f"{row.get('id')} flags")
        if not isinstance(row["duration_ms"], int):
            fail(f"{row.get('id')} duration_ms")
        if not isinstance(row["measurements"], dict):
            fail(f"{row.get('id')} measurements")
        if not isinstance(row["detail"], str) or not isinstance(row["id"], str):
            fail(f"{row.get('id')} text fields")
    summary = report["summary"]
    if not isinstance(summary, dict):
        fail("summary must be an object")
    for key in ("pass", "fail", "skip"):
        if summary.get(key) != counts[key]:
            fail(f"summary {key} {summary.get(key)} != {counts[key]}")
    print(f"schema ok {report['suite']} {summary}")


if __name__ == "__main__":
    main()
