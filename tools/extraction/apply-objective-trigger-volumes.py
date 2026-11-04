#!/usr/bin/env python3
"""Attach authored trigger boxes to an already projected objective runtime catalog.

This is the --trigger-volumes step of project-retail-objective-runtime.py for a
catalog that was projected earlier. The authored objective metadata must be the
exact file the runtime catalog records in source.authored_catalog_sha256; its
trigger links decide which boxes each interaction or demolition rule receives.
Output is game-derived data: write it outside the source checkout.
"""
import argparse
import json
from pathlib import Path
import runpy

TOOLS = Path(__file__).resolve().parent
RUNTIME = runpy.run_path(str(TOOLS / 'project-retail-objective-runtime.py'))
require, load, token = RUNTIME['require'], RUNTIME['load'], RUNTIME['token']
RULE_KINDS = ('interaction', 'demolition')


def apply(runtime, graph, graph_sha, volumes):
    require(runtime.get('format') == 'mag_retail_objective_catalog' and runtime.get('schema_version') == 1,
            'wrong objective runtime catalog identity')
    require(runtime['source']['authored_catalog_sha256'] == graph_sha,
            'authored objective metadata is not the runtime catalog source')
    require(runtime['map_name'] == graph['map']['map'], 'runtime catalog and metadata maps differ')
    rules = {}
    for row in runtime['objects']:
        kinds = [kind for kind in RULE_KINDS if kind in row]
        require(len(kinds) <= 1, 'objective has both interaction and demolition rules')
        if kinds:
            rules[token(row['token'])] = {kinds[0]: row[kinds[0]]}
    require(rules, 'runtime catalog has no interaction or demolition rules')
    RUNTIME['attach_trigger_volumes'](graph, rules, volumes)
    return runtime


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--runtime-catalog', type=Path, required=True, help='Projected objective runtime catalog')
    parser.add_argument('--catalog', type=Path, required=True, help='Authored objective metadata it was projected from')
    parser.add_argument('--trigger-volumes', type=Path, required=True, help='extract-retail-objective-volumes.py output')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    require(not args.output.resolve().is_relative_to(TOOLS.parents[1]), 'output must be outside the source checkout')
    require(args.output.resolve() != args.runtime_catalog.resolve(), 'output must not replace the input catalog')
    runtime, _ = load(args.runtime_catalog)
    graph, graph_sha = load(args.catalog)
    volumes, _ = load(args.trigger_volumes)
    data = apply(runtime, graph, graph_sha, volumes)
    args.output.write_text(json.dumps(data, indent=2, allow_nan=False) + '\n')
    boxes = sum(len(row[kind]['trigger_volumes']) for row in data['objects'] for kind in RULE_KINDS if kind in row)
    print(json.dumps({'map': data['map_name'], 'trigger_volumes': boxes}))


if __name__ == '__main__':
    main()
