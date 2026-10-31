# Contributing

Keep replacement implementation code and private runtime material separate.
Contribute original source, schemas, synthetic examples and synthetic tests.
Preserve upstream notices and identify changes to vendored code.

Do not add game executables, firmware, extracted resources or catalogs, raw
decompiled/generated game bodies, client packet captures, screenshots, account
records, credentials, certificate templates or private filesystem locations.
Renaming a capture or translating an extracted catalog into JSON does not make
it a synthetic fixture. The same rules apply to embedded strings and byte arrays.

Construct fixtures independently from a documented schema with invented values.
Document their origin. Strict JSON schemas can use hash-bound provenance in
`tools/synthetic-data.json`. Keep real-client acceptance tests and their inputs
in an access-controlled integration workspace.

Run the publication guard and affected synthetic checks before committing.
Describe the resulting behavior and its validation. Do not claim real-client
acceptance, gameplay performance or production readiness from synthetic tests.

New original project contributions are made under the root MIT license. Third-party
code keeps its own attribution and terms. No contribution may grant rights in
game resources that its author does not own.
