# Private runtime JSON bundles

Extracted game JSON is an external, private runtime input. Keep it out of this
repository, GHCR images, chart packages, ConfigMaps and public CI artifacts.
Use a private S3-compatible object store, including an existing RustFS service,
for the catalogs. The server's small configuration stays in a ConfigMap;
credentials and RSA material come from existing Kubernetes Secrets. Account and
character state belongs on the separate accounts PVC, never in a catalog bundle.

The chart supports an existing catalog PVC and `data.mode: s3`. In S3 mode each
Pod downloads one immutable archive, verifies it, and initializes its own catalog
emptyDir before application containers start. The same volume is mounted read
only at `/data` in the application containers. A roughly 57 MiB catalog set is
well above the [1 MiB ConfigMap limit](https://kubernetes.io/docs/concepts/configuration/configmap/);
only the small verifier program is carried by the chart's script ConfigMap.
RustFS exposes [S3-compatible object APIs](https://docs.rustfs.com/en/reference/s3-compatibility),
which fits the chart's private object-download workflow.

## Prepare and select inputs

Run the authorized extraction workflow outside every Git checkout. Use its real
catalog validation and the matching server version to check the extraction.
Synthetic tests in this repository prove tooling behavior, not compatibility
with an original game or semantic correctness of extracted data.

The packer requires an explicit relative file list. It does not recursively
collect a directory. The selection must contain regular UTF-8 JSON files with
safe relative paths. Symlink files or directories, duplicate JSON keys, NaN,
Infinity and overflowing floating point numbers are rejected. Archives, raw game
files, account/character stores, common credential/certificate/key filenames,
credential-like JSON keys and PEM private key/certificate content are rejected.
This screening is a guard against common mistakes, not a classifier for all
possible private material: operators must review the selected files.

Use Python 3.11 or newer; installation of Python packages is unnecessary. For
example, with private directories supplied in your shell:

```sh
python3 tools/data-bundle.py pack \
  --input-dir "$PRIVATE_EXTRACT_DIR" \
  --output "$PRIVATE_BUNDLE_DIR/catalogs-v2.tar.gz" \
  --include items.json \
  --include spawns.json \
  --include wire-profiles.json \
  --include skills.json \
  --include loadout-defaults.json \
  --include development-progression.json \
  --include training-completion.json \
  --include progression.json \
  --include retail-skills.json \
  --include retail-armory.json \
  --data-revision catalogs-v2 \
  --source-hash "$PRIVATE_SOURCE_SHA256"
```

Choose the exact filenames expected by your private extraction and server
configuration; the example names match the chart defaults. If optional gameplay
arguments refer to additional catalogs, select those too and configure their
paths. The packer does not discover server dependencies. As an alternative to
repeated `--include`, provide `--selection-manifest` with a private JSON file of
the form `{"files":["items.json","nested/spawns.json"]}`. Both selections may be
combined, but duplicate paths fail.

The output must be a new file outside the input directory and outside any Git
checkout. The tool never overwrites an archive and creates it with mode 0600.
It prints only SHA-256, file count and byte counts. Preserve that SHA-256 as the
deployment identity. `--data-revision` and `--source-hash` are optional provenance
strings carried inside the private manifest; they are operator assertions.
Identical inputs, selections and metadata produce identical archive bytes with
the same Python/compressor implementation. Timestamps, file owner IDs and source
file modes do not affect the archive.

## Upload and pin one object

Upload only to a private bucket using a credential profile outside the
repository. A content-derived key such as `catalogs/<SHA256>/bundle.tar.gz` is a
useful convention. Deny public access and avoid overwriting keys; enable object
versioning or retention where available. The tool does not upload anything.

For an operator-run AWS CLI upload, supply your endpoint, bucket and immutable
key without embedding credentials in the command:

```sh
aws --endpoint-url "$S3_ENDPOINT" s3 cp \
  "$PRIVATE_BUNDLE_DIR/catalogs-v2.tar.gz" \
  "s3://$PRIVATE_BUCKET/$IMMUTABLE_KEY"
```

Record the returned object version ID separately when using a versioned bucket.
Use TLS and normal certificate verification. RustFS uses the same chart S3
contract; its endpoint is nonsecret, while access credentials must remain in an
existing Secret. Give download credentials read access only to the relevant
private prefix, with version-read permission when pinning a version.

Update the deployment's private GitOps values with the complete identity:

```yaml
data:
  mode: s3
  revision: catalogs-v2
  s3:
    endpoint: https://objects.example.invalid
    region: us-east-1
    bucket: private-runtime-catalogs
    key: catalogs/REPLACE_WITH_SHA256/bundle.tar.gz
    versionId: "" # Set when pinning an object version.
    sha256: REPLACE_WITH_SHA256
    existingSecret: catalog-download-credentials
    forcePathStyle: true
```

The existing Secret supplies `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`, and
optionally `AWS_SESSION_TOKEN`. Do not put credentials in Helm values, an object
URL or a public Git repository. Use your cluster's external secret mechanism
when available. The object key, SHA-256 and optional version ID must describe
one archive. The chart ties the data identity to Pod rollout; change them as a
unit. Application image version and catalog revision are independent pins.

## Initialization and verification contract

The AWS CLI init container downloads the pinned object into the download volume
as `/download/bundle.tar.gz`. The next init container runs the exact canonical
program shipped in `charts/mag-recomp-server/files/data_bundle.py`:

```sh
python3 /scripts/data_bundle.py unpack \
  --archive /download/bundle.tar.gz \
  --sha256 REPLACE_WITH_SHA256 \
  --output-dir /catalogs
```

`/catalogs` is an already mounted, empty directory. Verification leaves that
mount root in place. It hashes the entire compressed archive before reading
tar contents, then stages all files under a temporary child directory. Only
after every file passes validation are the staged children promoted into the
volume. Files and the internal `.mag-data-manifest.json` become mode 0444. An
existing nonempty output, including a previously completed bundle, fails; use a
fresh Pod/emptyDir for a different catalog version. The container startup order
keeps applications from observing promotion in progress. An interrupted or
failed promotion must never be accepted as a complete bundle: recreate the Pod
if it leaves a nonempty volume. The init containers must be the only writers.

Manifest schema `mag-data-bundle`, version `1`, contains an exact file list with
per-file byte size and SHA-256, `data_revision`, `source_hash` and
`compatibility: operator-validated`. Unsupported versions or policies fail.
This policy means the operator is responsible for semantic compatibility; the
manifest itself does not prove it. The outer SHA-256 must come from trusted
private GitOps values: a hash copied from an untrusted object is not
authentication.

The verifier accepts bounded regular USTAR members only. It rejects traversal,
absolute paths, duplicate or unlisted members, links, devices, directories,
PAX/GNU extension records, size/hash mismatches, missing files, invalid JSON and
nonzero or excessive trailing data. It never calls `extractall` or executes any
archive content. Both CLI commands apply these configurable limits:

| Option | Default | Meaning |
| --- | ---: | --- |
| `--max-members` | 4096 | Catalog files, excluding the manifest |
| `--max-file-bytes` | 536870912 | Bytes in one catalog |
| `--max-total-bytes` | 2147483648 | Sum of catalog bytes |
| `--max-archive-bytes` | 2147483648 | Compressed archive bytes |

The manifest has a fixed 4 MiB ceiling. Archive hashing and tar traversal stream
from disk; JSON validation holds one bounded file and its parsed object in
memory. Set init-container memory for the largest real catalog's parsed shape,
and size the download/catalog volumes for archive, staging and extracted data.
Compressed size alone does not predict either requirement. Validate those limits
with real private inputs before deployment.

## Update and rollback

For each authorized catalog update: extract outside Git, run affected synthetic
checks and operator validation against real private data, pack a new archive,
upload it under a new private immutable key, then update the key/SHA-256/version
identity in private GitOps values. Review the rendered configuration and roll
out through the normal deployment workflow. Check init completion and server
readiness before doing any authorized game validation.

Rollback by restoring the old immutable object key, SHA-256 and optional version
ID, along with the application image pin required for that data. Retain old
objects while rollback remains possible. Catalog rollbacks do not roll back
accounts; account migration/backup policy is separate. No pack, upload, rollout
or game run is performed automatically by these instructions.
