#!/usr/bin/env python3
"""Pack private JSON inputs and verify/extract immutable bundles. Python 3.11+."""
import argparse
import gzip
import hashlib
import io
import json
import math
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import sys
import tarfile
import tempfile

MANIFEST = ".mag-data-manifest.json"
MANIFEST_LIMIT = 4 * 1024 * 1024
SCHEMA = "mag-data-bundle"
DENIED_NAMES = {"account", "accounts", "character", "characters", "credential",
                "credentials", "secret", "secrets", "certificate", "certificates",
                "cert", "certs", "key", "keys", "private", "users", "sessions",
                "password", "passwords", "tokens", "id_rsa", "id_ed25519"}
DENIED_KEYS = {"password", "passwordhash", "passwordsalt", "privatekey", "secretkey",
               "clientsecret", "accesstoken", "refreshtoken", "sessiontoken",
               "awsaccesskeyid", "awssecretaccesskey", "credentials"}


class BundleError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise BundleError(message)


def positive(value):
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("limit must be positive")
    return number


def safe_path(name):
    require(isinstance(name, str) and len(name.encode("utf-8")) <= 240,
            "invalid member path")
    require(bool(name) and not name.startswith("/") and "\\" not in name,
            "absolute or platform-specific member path")
    require(all(re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_.-]*", p)
                and p not in {".", ".."} for p in name.split("/")), "unsafe member path")
    require(PurePosixPath(name).suffix == ".json", "only JSON catalog files are allowed")
    for component in PurePosixPath(name).parts:
        tokens = set(re.split(r"[_.-]+", component.lower()))
        require(not tokens.intersection(DENIED_NAMES), "sensitive file path is forbidden")
    return name


def pairs_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON object key")
        result[key] = value
    return result


def parse_json(data, screen_secrets=False):
    try:
        value = json.loads(data.decode("utf-8"), object_pairs_hook=pairs_object,
                           parse_constant=lambda _: (_ for _ in ()).throw(BundleError("nonfinite JSON number")))
    except (UnicodeError, json.JSONDecodeError, RecursionError) as exc:
        raise BundleError("invalid UTF-8 JSON") from exc
    pending = [value]
    while pending:
        item = pending.pop()
        if isinstance(item, float):
            require(math.isfinite(item), "nonfinite JSON number")
        elif isinstance(item, dict):
            if screen_secrets:
                require(not any(re.sub(r"[^a-z0-9]", "", k.lower()) in DENIED_KEYS for k in item),
                        "credential-like JSON key is forbidden")
            pending.extend(item.values())
        elif isinstance(item, list):
            pending.extend(item)
        elif screen_secrets and isinstance(item, str):
            require(not re.search(r"-----BEGIN [A-Z0-9 ]*(?:PRIVATE KEY|CERTIFICATE)-----", item),
                    "private key or certificate content is forbidden")
    return value


def digest_file(handle, limit):
    digest = hashlib.sha256()
    size = 0
    while chunk := handle.read(1024 * 1024):
        size += len(chunk)
        require(size <= limit, "archive exceeds byte limit")
        digest.update(chunk)
    return digest.hexdigest(), size


def no_symlink_path(path):
    absolute = Path(os.path.abspath(path))
    require(not absolute.is_symlink(), "symlink path is forbidden")
    # Trusted CLI parents may use OS aliases such as /var -> /private/var.
    # Relative catalog components are checked separately below.
    return absolute.parent.resolve() / absolute.name


def open_regular(path):
    no_symlink_path(path)
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    handle = os.fdopen(descriptor, "rb")
    if not stat.S_ISREG(os.fstat(handle.fileno()).st_mode):
        handle.close()
        raise BundleError("input must be a regular file")
    return handle


def validate_manifest(value, args):
    require(isinstance(value, dict) and set(value) == {
        "schema", "version", "compatibility", "data_revision", "source_hash", "files"},
        "unsupported manifest fields")
    require(value["schema"] == SCHEMA and type(value["version"]) is int and value["version"] == 1,
            "unsupported bundle schema/version")
    require(value["compatibility"] == "operator-validated", "unsupported compatibility policy")
    for field in ("data_revision", "source_hash"):
        require(isinstance(value[field], str) and len(value[field]) <= 512 and
                all(ord(c) >= 32 for c in value[field]), "invalid manifest metadata")
    files = value["files"]
    require(isinstance(files, list) and 0 < len(files) <= args.max_members,
            "invalid file count or member limit exceeded")
    expected = {}
    total = 0
    for entry in files:
        require(isinstance(entry, dict) and set(entry) == {"path", "size", "sha256"},
                "invalid manifest file entry")
        name = safe_path(entry["path"])
        require(name not in expected, "duplicate manifest path")
        size = entry["size"]
        require(type(size) is int and 0 <= size <= args.max_file_bytes, "file exceeds byte limit")
        require(isinstance(entry["sha256"], str) and re.fullmatch(r"[0-9a-f]{64}", entry["sha256"]),
                "invalid file SHA-256")
        total += size
        require(total <= args.max_total_bytes, "bundle exceeds total byte limit")
        expected[name] = entry
    for name in expected:
        require(not any(str(parent) in expected for parent in PurePosixPath(name).parents),
                "conflicting member paths")
    return expected


def tar_add(tar, name, handle, size):
    info = tarfile.TarInfo(name)
    info.size = size
    info.mode = 0o444
    info.uid = info.gid = info.mtime = 0
    info.uname = info.gname = ""
    tar.addfile(info, handle)


def pack(args):
    source = no_symlink_path(args.input_dir)
    require(source.is_dir(), "input directory is missing")
    output = no_symlink_path(args.output)
    require(not output.exists(), "output archive already exists")
    require(output.parent.is_dir(), "output parent must exist")
    require(not output.is_relative_to(source), "output archive must be outside input directory")
    require(not any((parent / ".git").exists() for parent in output.parents),
            "private bundle output must be outside a Git checkout")
    names = list(args.include or [])
    if args.selection_manifest:
        with open_regular(args.selection_manifest) as handle:
            selection = parse_json(handle.read(MANIFEST_LIMIT + 1))
        require(isinstance(selection, dict) and set(selection) == {"files"} and
                isinstance(selection["files"], list), "selection manifest must contain only a files list")
        names.extend(selection["files"])
    require(0 < len(names) <= args.max_members, "explicit file selection required within member limit")
    names = [safe_path(name) for name in names]
    require(len(names) == len(set(names)), "duplicate input selection")
    names.sort()
    with tempfile.TemporaryDirectory(prefix="mag-bundle-pack-") as scratch:
        scratch = Path(scratch)
        entries = []
        total = 0
        for index, name in enumerate(names):
            candidate = source
            for component in PurePosixPath(name).parts:
                candidate = candidate / component
                require(not candidate.is_symlink(), "symlink input component is forbidden")
            with open_regular(source / name) as handle:
                data = handle.read(args.max_file_bytes + 1)
            require(len(data) <= args.max_file_bytes, "file exceeds byte limit")
            total += len(data)
            require(total <= args.max_total_bytes, "bundle exceeds total byte limit")
            parse_json(data, screen_secrets=True)
            (scratch / str(index)).write_bytes(data)
            entries.append({"path": name, "size": len(data), "sha256": hashlib.sha256(data).hexdigest()})
        manifest = {"schema": SCHEMA, "version": 1, "compatibility": "operator-validated",
                    "data_revision": args.data_revision, "source_hash": args.source_hash,
                    "files": entries}
        validate_manifest(manifest, args)
        encoded = json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode() + b"\n"
        require(len(encoded) <= MANIFEST_LIMIT, "manifest exceeds byte limit")
        descriptor, temporary = tempfile.mkstemp(prefix=".mag-bundle-", dir=output.parent)
        try:
            with os.fdopen(descriptor, "wb") as raw:
                with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0, compresslevel=9) as compressed:
                    with tarfile.open(fileobj=compressed, mode="w|", format=tarfile.USTAR_FORMAT) as tar:
                        tar_add(tar, MANIFEST, io.BytesIO(encoded), len(encoded))
                        for index, entry in enumerate(entries):
                            with (scratch / str(index)).open("rb") as handle:
                                tar_add(tar, entry["path"], handle, entry["size"])
            with open(temporary, "rb") as handle:
                checksum, size = digest_file(handle, args.max_archive_bytes)
            os.link(temporary, output)  # Exclusive publication; never replace an existing path.
        finally:
            os.unlink(temporary)
    print(json.dumps({"sha256": checksum, "files": len(entries), "data_bytes": total,
                      "archive_bytes": size}, sort_keys=True))


def exact_read(handle, size):
    data = handle.read(size)
    require(len(data) == size, "truncated archive")
    return data


def unpack(args):
    require(re.fullmatch(r"[0-9a-fA-F]{64}", args.sha256), "invalid archive SHA-256")
    output = no_symlink_path(args.output_dir)
    require(output.is_dir(), "output directory must already exist")
    require(not any(output.iterdir()), "output directory must be empty")
    with open_regular(args.archive) as archive:
        checksum, _ = digest_file(archive, args.max_archive_bytes)
        require(checksum == args.sha256.lower(), "archive SHA-256 mismatch")
        archive.seek(0)
        with tempfile.TemporaryDirectory(prefix=".mag-bundle-stage-", dir=output) as staged:
            staged = Path(staged)
            with gzip.GzipFile(fileobj=archive, mode="rb") as compressed:
                expected = None
                seen = set()
                members = 0
                while True:
                    block = exact_read(compressed, 512)
                    if block == b"\0" * 512:
                        require(exact_read(compressed, 512) == b"\0" * 512, "invalid tar end marker")
                        # Canonical tar padding is bounded; never consume an unbounded gzip tail.
                        tail = compressed.read(10241)
                        require(len(tail) <= 10240 and not tail.strip(b"\0"), "unexpected archive trailer")
                        break
                    members += 1
                    require(members <= args.max_members + 1, "archive member limit exceeded")
                    info = tarfile.TarInfo.frombuf(block, "utf-8", "strict")
                    require(info.type in (tarfile.REGTYPE, tarfile.AREGTYPE),
                            "only regular tar members are allowed (no links or extension records)")
                    require(info.name not in seen, "duplicate archive member")
                    seen.add(info.name)
                    if expected is None:
                        require(info.name == MANIFEST and 0 <= info.size <= MANIFEST_LIMIT,
                                "first archive member must be bounded manifest")
                        data = exact_read(compressed, info.size)
                        expected = validate_manifest(parse_json(data), args)
                    else:
                        safe_path(info.name)
                        require(info.name in expected, "unlisted archive member")
                        entry = expected[info.name]
                        require(info.size == entry["size"], "manifest file size mismatch")
                        data = exact_read(compressed, info.size)
                        require(hashlib.sha256(data).hexdigest() == entry["sha256"],
                                "manifest file SHA-256 mismatch")
                        parse_json(data, screen_secrets=True)
                    target = staged / info.name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with target.open("xb") as handle:
                        handle.write(data)
                    target.chmod(0o444)
                    require(not exact_read(compressed, (-info.size) % 512).strip(b"\0"), "invalid tar padding")
                require(expected is not None and seen == set(expected) | {MANIFEST},
                        "archive missing listed files")
            require(set(output.iterdir()) == {staged}, "output changed during bundle validation")
            promoted = []
            try:
                for child in sorted(staged.iterdir()):
                    require(not (output / child.name).exists(), "output changed during promotion")
                    child.rename(output / child.name)
                    promoted.append(output / child.name)
            except BaseException:
                for child in promoted:
                    if child.is_dir():
                        shutil.rmtree(child)
                    else:
                        child.unlink()
                raise
    print(json.dumps({"sha256": checksum, "files": len(expected)}, sort_keys=True))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for command in ("pack", "unpack"):
        sub = commands.add_parser(command)
        sub.add_argument("--max-members", type=positive, default=4096)
        sub.add_argument("--max-file-bytes", type=positive, default=512 * 1024 * 1024)
        sub.add_argument("--max-total-bytes", type=positive, default=2 * 1024**3)
        sub.add_argument("--max-archive-bytes", type=positive, default=2 * 1024**3)
        if command == "pack":
            sub.add_argument("--input-dir", type=Path, required=True)
            sub.add_argument("--output", type=Path, required=True)
            sub.add_argument("--include", action="append")
            sub.add_argument("--selection-manifest", type=Path)
            sub.add_argument("--data-revision", default="")
            sub.add_argument("--source-hash", default="")
        else:
            sub.add_argument("--archive", type=Path, required=True)
            sub.add_argument("--sha256", required=True)
            sub.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        (pack if args.command == "pack" else unpack)(args)
    except (BundleError, OSError, EOFError, tarfile.TarError, UnicodeError, ValueError) as exc:
        # JSON contents and credential values are deliberately never included.
        print("data-bundle: " + (str(exc) if isinstance(exc, BundleError) else type(exc).__name__), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
