"""Merge public chart settings with operator-owned stable RSA keys, without logging keys."""
import json
from pathlib import Path
import shutil
import sys


def main():
    public, private, output = map(Path, sys.argv[1:4])
    for name in ("scert-mas-certificate.bin", "scert-mls-certificate.bin"):
        if not (private / name).is_file() or not (private / name).stat().st_size:
            raise ValueError("runtime Secret is missing a required SCERT certificate template")
    if (private / "scert-mas-certificate.bin").stat().st_size < 0x1D4:
        raise ValueError("MAS SCERT certificate template must be at least 468 bytes")
    keys = json.loads((private / "medius-keys.json").read_text())
    if set(keys) != {"MPSKey", "DefaultKey"}:
        raise ValueError("medius-keys.json must contain only MPSKey and DefaultKey")
    for value in keys.values():
        if not isinstance(value, dict) or set(value) != {"n", "e", "d"}:
            raise ValueError("RSA keys require decimal-string n, e and d")
        if any(not isinstance(v, str) or not v.isdecimal() or int(v) <= 1 for v in value.values()):
            raise ValueError("RSA components must be positive decimal strings greater than one")
    for role in ("main", "maps"):
        target = output / role
        target.mkdir(parents=True, exist_ok=True)
        config = json.loads((public / (role + ".json")).read_text())
        config.update(keys)
        path = target / "medius.json"
        path.write_text(json.dumps(config))
        path.chmod(0o600)
        if (private / "db.config.json").exists():
            shutil.copyfile(private / "db.config.json", target / "db.config.json")
            (target / "db.config.json").chmod(0o600)


if __name__ == "__main__":
    main()
