#!/usr/bin/env python3
"""Project firearm-assist timing from neutral retail rules and its source literal."""
import argparse
import hashlib
import json
from pathlib import Path
import re

APPLICATION_ID = 20231
MEMBER = "settings/magrootsettings.lua"
TIMEOUT_KEY = "AssistTimeoutMs"
U32_MAX = (1 << 32) - 1


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key {key!r}")
        result[key] = value
    return result


def project(raw):
    source = json.loads(raw, object_pairs_hook=unique_object)
    if (type(source["schema_version"]) is not int or source["schema_version"] != 1
            or type(source["application_id"]) is not int
            or source["application_id"] != APPLICATION_ID):
        raise ValueError("expected retail rules schema 1 for application 20231")
    resource = source["resources"][MEMBER]
    member_bytes = resource["bytes"]
    member_sha = resource["sha256"]
    if (resource["member"] != MEMBER or type(member_bytes) is not int or member_bytes <= 0
            or not isinstance(member_sha, str) or not re.fullmatch(r"[0-9a-f]{64}", member_sha)):
        raise ValueError("invalid retail settings member provenance")
    rows = resource["parameters"]["$lua_table"]
    if not isinstance(rows, list) or any(not isinstance(row, dict) for row in rows):
        raise ValueError("expected a retail Lua table")
    matches = [(index, row) for index, row in enumerate(rows)
               if row.get("key") == "MAGCharacterStats"]
    if len(matches) != 1:
        raise ValueError("expected one MAGCharacterStats source entry")
    index, entry = matches[0]
    timeout = entry["value"][TIMEOUT_KEY]
    if type(timeout) is not int or not 0 <= timeout <= U32_MAX:
        raise ValueError(f"{TIMEOUT_KEY} is outside the u32 integer domain")
    source_path = f"/$lua_table/{index}/value/{TIMEOUT_KEY}"
    literal = resource["source_literals"][source_path]
    if (not isinstance(literal, str) or not re.fullmatch(r"[0-9]+", literal)
            or int(literal) != timeout):
        raise ValueError(f"{TIMEOUT_KEY} does not match its source literal")
    return (
        {"schema_version": 1, "application_id": APPLICATION_ID, "assist_timeout_ms": timeout},
        {"schema_version": 1, "input_sha256": hashlib.sha256(raw).hexdigest(),
         "member": MEMBER, "member_sha256": member_sha, "member_bytes": member_bytes,
         "source_path": source_path, "source_literal": literal,
         "source_literal_sha256": hashlib.sha256(literal.encode()).hexdigest()},
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--retail-rules", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--check", action="store_true", help="verify existing output bytes")
    args = parser.parse_args()
    try:
        paths = [path.resolve() for path in (args.retail_rules, args.out, args.receipt)
                 if path is not None]
        if len(paths) != len(set(paths)):
            raise ValueError("input and outputs must be distinct paths")
        rules, receipt = project(args.retail_rules.read_bytes())
        for path, document in ((args.out, rules), (args.receipt, receipt)):
            if path is None:
                continue
            encoded = (json.dumps(document, sort_keys=True, indent=2) + "\n").encode()
            if args.check:
                if path.read_bytes() != encoded:
                    raise ValueError(f"projection differs from {path}")
            else:
                path.write_bytes(encoded)
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(1, f"error: {error}\n")


if __name__ == "__main__":
    main()
