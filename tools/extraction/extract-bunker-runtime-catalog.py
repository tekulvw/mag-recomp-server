#!/usr/bin/env python3
"""Project retained, hashed prototype extraction evidence into runtime catalogs.

No game archives are changed. Pool indices remain provisional unless independently
verified; serialized ordinals must not be mistaken for embedded native object IDs.
"""
import argparse
import hashlib
import json
from pathlib import Path

FACTION = {"Valor": 0, "Raven": 1, "SVER": 2}
SQUAD = {"Alpha": 0, "Bravo": 1, "Charlie": 2, "Delta": 3}
MAPS = ("mag_unalaska_final", "mag_voz_final")


def extract(evidence, active_policy):
    sources = []

    def read(relative):
        path = evidence / relative
        raw = path.read_bytes()
        sources.append({"path": str(path.relative_to(evidence)), "sha256": hashlib.sha256(raw).hexdigest()})
        return json.loads(raw)

    bunker_evidence = read("bunker-exit-catalog/catalog.json")
    groups = read("spawn-catalog/squad-spawns.json")["groups"]
    verified_spawn = read("bunker-turret/verified-spawn66.json")
    objectives = read("objective-runtime/verified-objective-ids.json")
    pool_proof = read("bunker-damage-repair/spawn-pool-binding-proof.json")
    assert pool_proof["status"] == "native_static_initial_load"
    assert pool_proof["rule"].startswith("spawn_runtime_index = serialized_spawn_ordinal;")
    objective_ids = {o["name_token"]: o["index"] for o in objectives["objects"]}
    spawn_maps, bunker_maps = [], []
    vehicle_tokens = set()
    for name in MAPS:
        spawns = read(f"spawn-catalog/{name}.json")
        bunkers = read(f"bunker-exit-catalog/{name}.json")
        assert len(spawns["points"]) == pool_proof["maps"][name]["spawn_count"]
        owner = FACTION[spawns["map"]["owner_faction"]]
        bunker_by_token = {b["spawn_name_token"]: b for b in bunkers["bunkers"]}
        locations = []
        point_by_id = {}
        for point in spawns["points"]:
            ordinal = point["serialized_spawn_ordinal"]
            kind = "unsupported"
            if point["spawn_type"] == 0:
                kind = {0: "insertion", 1: "alternate"}.get(point["category"], "on_foot")
            elif point["name_token"] in bunker_by_token:
                assert point["vehicle_type"] == "DefensiveEmplacement" and point["vehicle_mountable"]
                vehicle_field = next(f for f in point["spawn_properties"]["fields"] if f["name"] == "VehicleType")
                vehicle_tokens.add(int(vehicle_field["raw_hex"], 16))
                kind = "bunker_passenger"
            loc = {"index": ordinal, "token": int(point["name_token"], 16), "kind": kind, "active": point["active"],
                   "position": dict(zip(("x", "y", "z"), point["position_xyz"]))}
            locations.append(loc)
            point_by_id[point["catalog_id"]] = (point, loc)
        bindings = []
        for group in (g for g in groups if g["map"] == name):
            team, platoon, squad = group["team"], group["platoon_id"], SQUAD[group["squad"]]
            available = []
            for slot, role in enumerate(("insertion", "alternate", "forward")):
                ref = group[role]
                point, loc = point_by_id[ref["catalog_id"]]
                assert ref["reference"]["slot"] == slot and point["category"] == slot
                assert ref["reference"]["point_token"] == point["name_token"]
                assert point["team"] == team
                if point["active"] and loc["kind"] != "unsupported":
                    available.append(loc["index"])
            assert available, f"no implemented spawn choices: {name}/{group}"
            bindings.append({"index": team * 16 + platoon * 4 + squad, "team": team,
                             "platoon": platoon, "squad": squad,
                             "faction": owner if team == 1 else None,
                             "available": available, "default_spawn": available[-1]})
        spawn_maps.append({"name": name, "spawn_count": len(locations), "locations": locations,
                           "owner_faction": owner, "runtime_indices_verified": True, "runtime_indices_basis": pool_proof["status"],
                           "squads": sorted(bindings, key=lambda s: s["index"])})
        records = []
        for bunker in bunkers["bunkers"]:
            assert len(bunker["squad_links"]) == len(bunker["bunker_objectives"]) == 1
            record = dict(bunker)
            link = bunker["squad_links"][0]
            assert link["team"] == 1 and link["slot"] == 2 and FACTION[bunker["faction"]] == owner
            record["faction_id"] = owner
            record["squad_index"] = 16 + link["platoon_id"] * 4 + SQUAD[link["squad"]]
            record["spawn_candidate_index"] = bunker["spawn_catalog_ordinal"]
            record["spawn_runtime_index"] = bunker["spawn_catalog_ordinal"]
            record["runtime_identity_status"] = pool_proof["status"]
            if name == "mag_unalaska_final" and bunker["spawn_name_token"] == verified_spawn["name_token"]:
                assert bunker["spawn_catalog_ordinal"] == verified_spawn["index"]
                record["spawn_runtime_index"] = verified_spawn["index"]
                record["runtime_identity_status"] = "independently_live_verified"
            record["bunker_objectives"] = [dict(o) for o in bunker["bunker_objectives"]]
            if name == "mag_unalaska_final":
                for objective in record["bunker_objectives"]:
                    objective["runtime_index"] = objective_ids[objective["name_token"]]
            records.append(record)
        bunker_maps.append({"name": name, "owner_faction": owner, "spawn_count": len(locations),
                            "runtime_indices_verified": True, "runtime_indices_basis": pool_proof["status"], "source": bunkers["source"], "bunkers": records})
    spawn_catalog = {"format": "mag_spawn_catalog", "schema_version": 1,
                     "source": json.dumps(sources, separators=(",", ":")),
                     "active": active_policy, "maps": spawn_maps}
    active_map = next((m for m in spawn_maps if m['name'] == active_policy.get('map_name')), None)
    assert active_map is not None, 'active map absent from source exports'
    active_squad = next((sq for sq in active_map['squads'] if sq['index'] == active_policy.get('squad_index')), None)
    assert active_squad is not None, 'active squad absent from source exports'
    assert set(active_policy) == {'map_name', 'squad_index', 'default_spawn', 'available'}
    assert active_policy['available'] and all(v in active_squad['available'] for v in active_policy['available'])
    assert active_policy['default_spawn'] in active_policy['available']
    faction_definitions = [dict(v, faction_id=FACTION[k]) for k, v in bunker_evidence["faction_property_variants"].items()]
    assert len(vehicle_tokens) == 1
    bunker_catalog = {"vehicle_type_token": vehicle_tokens.pop(), "format": "mag_bunker_catalog", "schema_version": 1, "source": sources,
                      "scope": "Bundled October 2009 prototype maps only; authored pre-terrain-snap placements",
                      "configured_but_not_bundled": bunker_evidence["configured_but_not_bundled"],
                      "faction_definitions": sorted(faction_definitions, key=lambda f: f["faction_id"]),
                      "maps": bunker_maps,
                      "runtime_pool_binding_proof": pool_proof,
                      "limits": [limit for limit in bunker_evidence["limits"]
                                 if not limit.startswith("Authored ordinals are not runtime pool IDs;")] + [
                          "Native static pool indices assume fresh successful initial map load with no prior allocations; dynamic recreation, failed allocations and retail are outside this proof.",
                          "Only Alyeska Alpha spawn66 has independent live spawn identity evidence; all other points are bound by native static initial-load proof.",
                          "All 16 Alyeska structural objective IDs have independent live evidence; Arulkum IDs remain unknown.",
                          "Default squad choice prefers the final implemented authored slot; this is local policy, not a traced native selection rule.",
                          "Attacking faction is match dependent; map-owned defending bunker placements must not be relabeled as another faction."]}
    return {"prototype-spawns.json": spawn_catalog, "prototype-bunkers.json": bunker_catalog}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True, help="external export bundle; see extraction documentation")
    parser.add_argument("--active-policy", type=Path, required=True, help="external active map/squad/default_spawn/available object")
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--check", action="store_true", help="Verify checked-in deterministic projection without writing")
    args = parser.parse_args()
    if args.output_dir.resolve().is_relative_to(args.evidence.resolve()):
        parser.error('output must be outside the evidence input directory')
    args.output_dir.mkdir(parents=True, exist_ok=True)
    for filename, catalog in extract(args.evidence, json.loads(args.active_policy.read_text())).items():
        path = args.output_dir / filename
        rendered = json.dumps(catalog, indent=2) + "\n"
        if args.check:
            if path.read_text() != rendered:
                raise SystemExit(f"catalog differs from retained evidence: {path}")
        else:
            path.write_text(rendered)
        print(f"{'verified' if args.check else 'wrote'} {path}")


if __name__ == "__main__":
    main()
