import copy
import hashlib
import json
from pathlib import Path
import runpy
import struct
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "tools/project-retail-human-resupply.py"
CLV = runpy.run_path(str(ROOT / "tools/extraction/extract-retail-spawns.py"))
INSTANCE_CLASS_TOKEN = 0x12001000
SUPPLY_TOKEN = 0x12002000


def encoded_string(value):
    raw = value.encode() + b"\0"
    return struct.pack(">I", len(raw)) + raw


def encoded_fields(fields):
    return b"".join(b"\x01" + encoded_string(name) + encoded_string(kind)
                    + struct.pack(">I", 4 + len(raw)) + raw
                    for name, kind, raw in fields) + b"\0"


def prefab_tree():
    raw = bytearray(struct.pack(">I", 3))

    def put_node(prefab, name, children, center=(1, 2, 3), parent_shift=0):
        offset = len(raw)
        fields = [("Name", "util::Token", struct.pack(">I", name)),
                  ("ObjectID", "uint32", struct.pack(">I", 0xFFFFFFFF))]
        for prefix, values in (("Translate", center), ("Rotate", (0, 90, 0)), ("Scale", (6, 4, 2))):
            fields.extend((prefix + axis, "float", struct.pack(">f", value))
                          for axis, value in zip("XYZ", values))
        body = struct.pack(">II", 0xDEADBEEF, 3) + encoded_string("InstanceProperties") + encoded_fields(fields)
        end = offset + 24 + len(body)
        raw.extend(struct.pack(">IIIIII", 5, prefab, 7, INSTANCE_CLASS_TOKEN, end, 0) + body)
        matrix = (1, 0, 0, parent_shift, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)
        raw.extend(bytes(8) + b"\x01" + struct.pack(">IIIIII", 0x10001, end + 101, 7, end + 29, 0, 0))
        raw.extend(struct.pack(">17f", *matrix, 1) + struct.pack(">IBI", 0x10001, 0, children))
        return offset + 12

    first_parent = put_node(0, 0x100, 1)
    first = put_node(SUPPLY_TOKEN, 0xEF, 0)
    put_node(0x12003000, 0x200, 0)
    put_node(0, 0x300, 1)
    second = put_node(SUPPLY_TOKEN, 0xA5, 0, (7, 8, 9))
    tokens = CLV["RetailTokens"]({INSTANCE_CLASS_TOKEN: "InstanceProperties"})
    clv = CLV["Clv"](bytes(raw), tokens)
    return clv, clv.tables("InstanceProperties"), (first_parent, first, second)


class HumanStaticSupplyProjectorTests(unittest.TestCase):
    def setUp(self):
        self.tool = runpy.run_path(str(SCRIPT))
        self.definition = {
            "token": SUPPLY_TOKEN, "offset": 411, "sha256": "b" * 64,
            "tick_rate_seconds": 1.25, "percent_refresh_rate": 13.5,
        }
        self.native = {"family": 42, "pool_capacity": 32, "start_tolerance_m": 1.5,
                       "unit_half_extent": 0.5}

    def providers(self, clv, rows):
        return self.tool["project_providers"](clv, rows, self.definition, self.native)

    def test_initial_target_follows_prefab_callback_order_and_each_authored_box(self):
        clv, rows, (_, first, second) = prefab_tree()
        result = self.providers(clv, list(reversed(rows)))
        self.assertEqual([p["packed_target"] for p in result], [0x2A800000, 0x2A800001])
        self.assertEqual([p["source"]["instance_offset"] for p in result], [first, second])
        self.assertEqual([p["source"]["instance_name_token"] for p in result], [0xEF, 0xA5])
        self.assertEqual([p["source"]["prefab_path"] for p in result], [[0, 0], [2, 0]])
        self.assertEqual(result[0]["box"]["center"], [1.0, 2.0, 3.0])
        self.assertEqual(result[1]["box"]["center"], [7.0, 8.0, 9.0])
        self.assertEqual(result[0]["box"]["scales_xyz"], [6.0, 4.0, 2.0])
        self.assertAlmostEqual(result[0]["box"]["axes"][0][2], -1.0)
        self.assertEqual(result[0]["tick_rate_seconds"], 1.25)
        self.assertEqual(result[0]["percent_refresh_rate"], 13.5)
        self.assertEqual(result[0]["box"]["start_tolerance_m"], 1.5)

    def test_refuses_incomplete_source_tree_or_unknown_parent_transform(self):
        clv, rows, (parent, _, _) = prefab_tree()
        with self.assertRaisesRegex(ValueError, "prefab coverage"):
            self.providers(clv, rows[:2] + rows[3:])
        parent_end = int(next(row["end"] for row in rows if int(row["offset"], 16) == parent), 16)
        raw = bytearray(clv.b)
        struct.pack_into(">f", raw, parent_end + 33 + 12, 2.0)
        moved = CLV["Clv"](bytes(raw), clv.tokens)
        with self.assertRaisesRegex(ValueError, "identity parent"):
            self.providers(moved, moved.tables("InstanceProperties"))

    def test_refuses_duplicate_name_and_explicit_level_object_id(self):
        clv, rows, (_, first, second) = prefab_tree()
        for offset, field, value, error in (
            (second, "Name", 0xEF, "unique authored Name"),
            (first, "ObjectID", 5, "automatic LevelObject ID"),
        ):
            with self.subTest(field=field):
                raw = bytearray(clv.b)
                row = next(row for row in rows if int(row["offset"], 16) == offset)
                location = next(int(f["offset"], 16) for f in row["fields"] if f["name"] == field)
                struct.pack_into(">I", raw, location, value)
                changed = CLV["Clv"](bytes(raw), clv.tokens)
                with self.assertRaisesRegex(ValueError, error):
                    self.providers(changed, changed.tables("InstanceProperties"))

    def test_definition_keeps_two_authored_rates_and_refuses_duplicates(self):
        fields = [("TickRate", "float", struct.pack(">f", 1.25)),
                  ("PercentRefreshRate", "float", struct.pack(">f", 13.5)),
                  ("UsesNonUniformScale", "bool", b"\x01")]
        body = struct.pack(">III", 0xDEADBEEF, 3, 0) + b"SupplyPointObjectProperties\0"
        body += encoded_fields(fields)
        raw = struct.pack(">IIII", 7, 0x12004000, 16 + len(body), SUPPLY_TOKEN) + body
        parse = self.tool["parse_definition"]
        definition = parse(CLV["Clv"](raw, {}))
        self.assertEqual(definition["token"], SUPPLY_TOKEN)
        self.assertEqual(definition["tick_rate_seconds"], 1.25)
        self.assertEqual(definition["percent_refresh_rate"], 13.5)
        with self.assertRaisesRegex(ValueError, "unique authored static supply definition"):
            parse(CLV["Clv"](raw + struct.pack(">IIII", 7, 0x12004000, len(raw) * 2, SUPPLY_TOKEN) + body, {}))

    def test_refuses_profile_from_another_source_or_mode(self):
        source = {"map": self.tool["MAP"], "level_archive_sha256": self.tool["LEVEL_SHA"],
                  "level_clv_sha256": self.tool["CLV_SHA"], "native_elf_sha256": self.tool["RETAIL_SHA"]}
        spawn = {"format": "mag_spawn_catalog", "schema_version": 1,
                 "active": {"map_name": self.tool["MAP"]},
                 "match_profile": {"game_type": 1, "attacking_faction": 0}, "source": json.dumps(source)}
        world = {"format": "mag_collision_world", "schema_version": 1, "coordinate_space": "world",
                 "metadata": {"map_name": self.tool["MAP"], "build": "retail-bcus98110-01.00",
                              "provenance": ["Original CLV SHA256 " + self.tool["CLV_SHA"]]}}
        encode = lambda value: json.dumps(value).encode()
        validate = self.tool["validate_profile"]
        result = validate(encode(spawn), encode(world))
        self.assertEqual(result["spawn_sha256"], hashlib.sha256(encode(spawn)).hexdigest())
        for candidate, path, value, error in (
            ("spawn", ("match_profile", "game_type"), True, "Panama Acquisition"),
            ("spawn", ("source",), json.dumps(dict(source, level_clv_sha256="f" * 64)), "spawn catalog source"),
            ("world", ("metadata", "provenance"), ["original source unknown"], "collision world source"),
        ):
            with self.subTest(path=path):
                inputs = copy.deepcopy({"spawn": spawn, "world": world})
                current = inputs[candidate]
                for key in path[:-1]:
                    current = current[key]
                current[path[-1]] = value
                with self.assertRaisesRegex(ValueError, error):
                    validate(encode(inputs["spawn"]), encode(inputs["world"]))

    def test_new_output_is_exactly_checkable_and_cannot_overwrite(self):
        write = self.tool["write_output"]
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "catalog.json"
            write(output, b"one", check=False)
            write(output, b"one", check=True)
            with self.assertRaisesRegex(ValueError, "projected bytes differ"):
                write(output, b"two", check=True)
            with self.assertRaises(FileExistsError):
                write(output, b"two", check=False)
            self.assertEqual(output.read_bytes(), b"one")
