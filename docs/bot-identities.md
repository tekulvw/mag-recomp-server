# Operator-supplied bot identities

The gameplay server uses stable synthetic names by default. To supply a private identity list, pass `--bot-names /absolute/path/names.json`. The file is loaded and validated before log creation or socket binding, including with `--validate-catalogs`. Explicit missing or invalid input fails startup; it never falls back silently.

```json
{"entries":[{"name":"Synthetic Pilot Alpha"},{"name":"Synthetic Pilot Bravo"}]}
```

The document must contain only `entries`, with 1–256 entries. Each name must be 1–32 printable ASCII bytes, with no surrounding whitespace; names must be unique ignoring ASCII case. Entry metadata such as `provenance` and `source` is permitted and ignored by the runtime. The file is limited to 128 KiB. Ordering controls the stable name indices. Supply at least as many names as `--bot-fill-total`; insufficient capacity fails admission rather than duplicating identities or wrapping indices.

Catalogs belong to individual engine resources through `EngineResources::with_bot_names`; there is no global override. The selected name is retained in the bot's owned state for client character snapshots and audit records, including after respawn. Startup diagnostics report the input path and count without dumping the list. Names are presentation only and do not change team, squad, account, equipment, combat or routing identity.

For Helm, set `data.files.botNames: operators/bot-names.json` and include that JSON member in the private data bundle, or provide it at that relative path on the data PVC. The chart passes `--bot-names /data/operators/bot-names.json`. Leave the value empty to keep synthetic defaults. The normal versioned bundle download and hash verification cover this file; no separate downloader or credentials are needed. Use a gameplay release containing the loader; version 0.2.0 does not support the flag. Keep private identity lists outside public Git, images and chart ConfigMaps.

All public examples and tests use synthetic names. An operator's credits-based list is operator configuration; this feature makes no claim that such a list was extracted from a game archive.
