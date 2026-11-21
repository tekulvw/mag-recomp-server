import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "tools/project-retail-firearm-assist-rules.py"
MEMBER = "settings/magrootsettings.lua"
POINTER = "/$lua_table/1/value/AssistTimeoutMs"


def fixture(value=713):
    return {
        "schema_version": 1, "application_id": 20231,
        "resources": {MEMBER: {
            "member": MEMBER, "bytes": 38, "sha256": "a" * 64,
            "parameters": {"$lua_table": [
                {"key": "SyntheticOther", "value": {}},
                {"key": "MAGCharacterStats", "value": {"AssistTimeoutMs": value}},
            ]},
            "source_literals": {POINTER: str(value)},
        }},
    }


class ProjectFirearmAssistRules(unittest.TestCase):
    def invoke(self, source, check=False):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            raw, out, receipt = (path / name for name in ("input", "out", "receipt"))
            raw.write_text(source if isinstance(source, str) else json.dumps(source))
            command = [sys.executable, str(SCRIPT), "--retail-rules", str(raw),
                       "--out", str(out), "--receipt", str(receipt)]
            result = subprocess.run(command, capture_output=True, text=True)
            if check and result.returncode == 0:
                replay = subprocess.run(command + ["--check"], capture_output=True, text=True)
                self.assertEqual(replay.returncode, 0, replay.stderr)
                receipt.write_text("{}")
                replay = subprocess.run(command + ["--check"], capture_output=True, text=True)
                self.assertNotEqual(replay.returncode, 0)
            return (result, json.loads(out.read_text()) if out.exists() else None)

    def test_projects_full_unsigned_range_and_reproduces(self):
        for value in (0, 713, 2**32 - 1):
            with self.subTest(value=value):
                result, output = self.invoke(fixture(value), check=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(output, {"schema_version": 1, "application_id": 20231,
                                          "assist_timeout_ms": value})

    def test_rejects_inconsistent_or_invalid_source(self):
        cases = [fixture(value) for value in (True, -1, 2**32, 713.0)]
        for variant in ("duplicate", "missing", "wrong_literal", "wrong_member"):
            source = fixture()
            resource = source["resources"][MEMBER]
            table = resource["parameters"]["$lua_table"]
            if variant == "duplicate":
                table.append(table[-1])
            elif variant == "missing":
                table[-1]["value"].clear()
            elif variant == "wrong_literal":
                resource["source_literals"][POINTER] = "714"
            else:
                resource["member"] = "wrong"
            cases.append(source)
        cases.append(json.dumps(fixture()).replace('"AssistTimeoutMs": 713',
                                                    '"AssistTimeoutMs": 714, "AssistTimeoutMs": 713'))
        for index, source in enumerate(cases):
            with self.subTest(index=index):
                result, output = self.invoke(source)
                self.assertNotEqual(result.returncode, 0, result.stderr)
                self.assertIn("error:", result.stderr)
                self.assertIsNone(output)
