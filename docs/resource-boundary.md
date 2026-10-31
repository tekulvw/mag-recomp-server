# Public code and private runtime resources

The public repository owns the replacement implementation: Rust gameplay and
protocol processing, Horizon integration, catalog readers, generic extraction
interfaces, build tools, and synthetic tests. A private integration workspace
can pin this repository at an exact commit and provide the resources it needs.

The private workspace owns extracted game catalogs, collision/navigation worlds,
certificate templates and configured keys, account databases, runtime logs,
client captures and acceptance tests, research exports and deployment settings.
These resources must not be copied into public source trees or container images.

The initial public extraction has independent Git history. The pre-existing
research repository and its complete history remain intact. Its deployments
and checkout paths are not switched by preparing the public source repository.

Public synthetic wire profiles and bot policy inputs are independently authored
from validators. Their values support public tests and compilation; they do not
replace complete runtime catalogs. Hash-bound descriptions of strict-schema
JSON are recorded in `tools/synthetic-data.json`.

Game-specific type names, packet identifiers and independently written codec
logic belong with the implementation. Copied original programs, packet blobs,
data extracted from game assets and production identities belong outside the
public boundary, including when embedded inside source files.

An eventual integration cutover should use one canonical public source checkout
at a pinned revision, supply resources by explicit paths, and run private
acceptance checks separately. Avoid maintaining two editable implementation
copies or copying private inputs into the public tests to make them pass.
