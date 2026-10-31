#!/usr/bin/env python3
"""Host entrypoint for the exact verifier shipped by the Helm chart."""
from pathlib import Path
import runpy

runpy.run_path(str(Path(__file__).resolve().parents[1] / "charts" /
                   "mag-recomp-server" / "files" / "data_bundle.py"), run_name="__main__")
