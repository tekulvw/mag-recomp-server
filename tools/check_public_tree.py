"""Check the public source boundary without reading private inputs.

This is a publication tripwire, not a substitute for reviewing provenance.
Only tracked files are inspected in a Git checkout; a plain export is scanned
recursively so the same checks work before the first local commit.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


FORBIDDEN_SUFFIXES = {
    ".bin", ".elf", ".self", ".sprx", ".rpx", ".iso", ".pup", ".pkg",
    ".pcap", ".pcapng", ".log", ".pem", ".key", ".pfx", ".p12", ".cer", ".crt", ".der",
    ".dll", ".exe", ".dylib", ".so", ".a", ".o", ".zip", ".gz",
    ".png", ".jpg", ".jpeg", ".mp4", ".sqlite", ".db", ".bundle",
}
PRIVATE_REFERENCE = re.compile(
    r"client" + r"-evidence|mag" + r"-connection-lab|mag" + r"-investigation|"
    r"(?:github\.com[:/][^\s/]+/)?mag" + r"_server(?:[/\.\s]|$)|"
    r"/Users/[\w.-]+(?:/|(?=[\s\"']|$))|/home/[\w.-]+/|/private/tmp/[\w.-]+|"
    r"[A-Za-z]:[\\/]+(?:Users|Documents and Settings)[\\/]+|"
    r"(?:~" + r"/|\$(?:HOME|\{HOME\})/|%USERPROFILE%[\\/])|"
    r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"
)
OPAQUE_HEX = re.compile(r"[\"'](?:[0-9a-fA-F]{2}){128,}[\"']")
OPAQUE_BASE64 = re.compile(r'[\"\'](?:[A-Za-z0-9+/]{256,}={0,2})[\"\']')
OPAQUE_DECIMAL = re.compile(r'[\"\'][0-9]{100,}[\"\']')
PRIVATE_LAN = re.compile(r'(?<![\w.])(?:192\.168|10\.(?:\d{1,3})|172\.(?:1[6-9]|2\d|3[01]))\.(?:\d{1,3})\.(?:\d{1,3})(?![\w.])')
MAP_DATA_HELPER = re.compile(r'\b(?:alyeska|arulkum)_(?:initial|alpha|valor)\s*\(')
SKIP_DIRECTORIES = {".git", "target", "obj", "__pycache__"}


def source_files(root: Path):
    # Only use an index owned by this exact root, not a parent workspace.
    if (root / ".git").exists():
        result = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z"], capture_output=True, check=True
        )
        names = result.stdout.decode().split("\0")
        if any(names):
            return [root / name for name in names if name]
    return [
        path for path in root.rglob("*")
        if not any(part in SKIP_DIRECTORIES for part in path.relative_to(root).parts)
        and (path.is_file() or path.is_symlink())
    ]


def check_tree(root: Path) -> list[str]:
    root = Path(root).absolute()
    errors = []
    provenance_path = root / "tools/synthetic-data.json"
    try:
        synthetic = json.loads(provenance_path.read_text()) if provenance_path.exists() else {}
    except (OSError, ValueError):
        synthetic = {}
        errors.append("tools/synthetic-data.json: invalid provenance manifest")
    for path in sorted(source_files(root)):
        relative = path.relative_to(root).as_posix()
        if path.is_symlink():
            errors.append(f"{relative}: symlinks are outside the public boundary")
            continue
        if path.suffix.lower() in FORBIDDEN_SUFFIXES or ".private." in path.name:
            errors.append(f"{relative}: private material or binary artifact extension")
            continue
        if path.name == ".env" or any(p in {"local", "captures", "decompiled"} for p in path.relative_to(root).parts):
            errors.append(f"{relative}: private runtime/research path")
            continue
        try:
            data = path.read_bytes()
            content = data.decode("utf-8-sig")
        except (OSError, UnicodeError):
            errors.append(f"{relative}: not a readable UTF-8 source file")
            continue
        if b"\0" in data:
            errors.append(f"{relative}: binary content")
        if len(data) > 1_048_576:
            errors.append(f"{relative}: exceeds the public source file size limit")
        if PRIVATE_REFERENCE.search(content):
            errors.append(f"{relative}: private path, research reference or private-key material")
        if PRIVATE_LAN.search(content):
            errors.append(f"{relative}: concrete private LAN address; use explicit configuration or loopback")
        if relative.startswith("gameplay/src/") and MAP_DATA_HELPER.search(content):
            errors.append(f"{relative}: known map-specific initialization helper; use external catalog data")
        if OPAQUE_HEX.search(content) or OPAQUE_BASE64.search(content) or OPAQUE_DECIMAL.search(content):
            errors.append(f"{relative}: opaque embedded byte image; use synthetic constructions")
        if relative.startswith("gameplay/data/") and path.suffix == ".json":
            try:
                document = json.loads(content)
                provenance = " ".join(str(document.get(k, "")) for k in ("source", "provenance", "description"))
                entry = synthetic.get(relative, {})
                certified = (
                    isinstance(entry, dict)
                    and "synthetic" in str(entry.get("source", "")).lower()
                    and entry.get("sha256") == hashlib.sha256(data).hexdigest()
                )
                if "synthetic" not in provenance.lower() and not certified:
                    errors.append(f"{relative}: catalog must declare its synthetic provenance")
            except (ValueError, AttributeError):
                errors.append(f"{relative}: invalid synthetic catalog document")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    errors = check_tree(args.root)
    if errors:
        print("Public tree check FAILED:")
        print("\n".join(errors))
        return 1
    print(f"Public tree check passed ({len(source_files(args.root))} source files).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
