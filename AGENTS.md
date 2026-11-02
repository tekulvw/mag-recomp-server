# Public replacement server rules

This repository is named `mag-recomp-server`. Keep that name and its configured
origin. It contains replacement implementation source and synthetic tests.

- Never add original game files, extracted catalogs, raw captures, decompiled or
  generated game code, accounts, private credentials or certificate templates.
- Public builds and tests must work without an access-controlled workspace.
- Runtime resources are explicit external inputs. Missing inputs must fail
  clearly; never invent a world or claim game compatibility from synthetic data.
- Keep upstream licenses, notices and source attribution.
- Run `python3 tools/check_public_tree.py` and the affected synthetic checks.
- Releases require the human maintainer's live-match review and manual release
  action. Agents must not merge release PRs, create release tags, publish releases
  or initiate release publication on the maintainer's behalf. Passing checks or
  source-integration approval is not release authorization. Release Please may
  prepare an open candidate PR; agents must not enable its automatic merge.
- Local-only preparation is authorized. Do not push, publish, deploy, launch a
  game or resume unrelated project work without explicit user authorization.
