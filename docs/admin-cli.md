# Account and character maintenance (`mag-admin`)

The medius image includes `mag-admin`, an operator CLI for the local character
store. It runs inside the main medius container, reads the same `medius.json`
and catalog environment as the server, and changes the store only through the
server's own code: the same file lock, XP/level rules and atomic commit. It
starts no listeners and never rewrites `medius.json`.

```sh
M=deploy/<release>-mag-recomp-server-medius
kubectl exec -n <namespace> $M -c medius -- mag-admin accounts list
kubectl exec -n <namespace> $M -c medius -- mag-admin character show <account>
kubectl exec -n <namespace> $M -c medius -- mag-admin character set-level <account> 3           # dry run
kubectl exec -n <namespace> $M -c medius -- mag-admin character set-level <account> 3 --apply
```

## Commands

| Command | Effect |
| --- | --- |
| `accounts list` | Every account's character with level, XP, skill points and session state (`online`, `offline`, or `unknown` when no session marker exists). |
| `character show <account>` | One character in detail, including the XP needed for the next level. |
| `character set-level <account> <level> [--apply]` | Raises the character to `<level>` the way earned progression would. XP becomes that level's catalog threshold, and each gained level's `SkillPointsGained` is added. Without `--apply` it only prints the planned change. |

Exit codes: `0` success or dry run, `1` refused or failed, `2` usage or
configuration error. `--config DIR` selects a medius config directory other than
`/config/main`.

## Safety rules

- **Catalog-consistent.** The server refuses to load a store whose level doesn't
  match `LevelForExperience(experience)`, so levels are only set through the
  progression catalog. A store without a progression catalog is refused.
- **Raise only.** Lowering a level is refused, because skill points may already
  have been spent.
- **Logged-out accounts only.** Each medius process publishes its live login
  leases (pending and bound tickets, up to 120 s each) to
  `$MAG_ACTIVE_ACCOUNTS_DIR/mag-active-accounts.<role>.json` (default `/tmp`).
  `--apply` is refused while the account holds a live lease, and also when no
  marker exists, since session state can't be confirmed then. Dry runs still work
  and print a warning.
- **Backed up.** Before writing, the current store is copied to
  `.maintenance-backups/characters.before-admin-set-level-<account>-<level>-<sha12>.json`
  next to the store. To revert, copy that file back over the store while the
  account is logged out.
- **Serialized with the server.** Changes happen under the store's own lock, so a
  concurrent reward or loadout save is applied before or after, never lost.

Medius re-reads the store at each login, so the client shows the new level on
its next login; no restart is needed.
