"""Validate an externally produced compiled-placement witness against its input ELF.

This does not regenerate or independently verify the native analysis in that witness.
"""
import json
from retail_terrain import RETAIL_SHA, require, sha

def verify_placement(proof_path, retail_path):
    proof = json.loads(proof_path.read_text())
    require(proof.get('schema') == 'mag_retail_compiled_placement_v1', 'unsupported external placement proof')
    require(sha(retail_path.read_bytes()) == RETAIL_SHA == proof.get('source_elf_sha256'), 'external placement ELF mismatch')
    require(proof.get('functions') and proof.get('call_chain'), 'missing external placement analysis')
    return proof
