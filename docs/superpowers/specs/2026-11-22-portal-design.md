# Player accounts portal (magtest.online)

Date: 2026-11-22. Status: phase 1 (the website) in progress.

## Why

Today any client can sign in as any name: the game's NP authenticate message carries the
player name as plain text, and Medius trusts it (`MagDevelopmentAccountPolicy`). The
maintainer wants all three of:

1. **Name ownership** — nobody can sign in as someone else.
2. **Bans** — an operator can stop an account from playing.
3. **Progress that follows the player** — the same account, and so the same characters, on
   any machine.

## Shape

```
browser ──Google sign-in──▶ portal (magtest.online) ◀── launcher: device link, then tickets
                                │ signs                         │
                                ▼                               ▼
                      short-lived game ticket ──▶ game ──NP authenticate──▶ Medius verifies
```

1. A player signs in to **magtest.online** with Google and claims one player name.
2. The launcher links once with a short code (device flow) and stores a device token.
3. Before each launch the launcher trades the device token for a **game ticket**: a ~150-byte
   binary record of account ID, player name, audience, expiry and a reuse-limited nonce, signed
   by the portal (ECDSA P-256). See [portal-ticket.md](../../portal-ticket.md).
4. The runtime's local NP ticket hands those bytes to the game, which sends them in its own
   NP authenticate message. Medius checks the signature with the portal's public key, and the
   expiry, audience and nonce, then binds the session to that account.

Medius never calls the portal during login, so login keeps working while the website is down.

## Phases

1. **Portal (this phase):** Google sign-in, name claim, device link, ticket issuing, admin
   CLI (ban, reserve names), ticket format spec with shared test vectors.
2. **Game check:** confirm the game forwards a ~150-byte ticket unchanged (no PSN parsing).
   The ticket format may change if it does not.
3. **Medius:** verify tickets, limit reuse (3 uses per service, one live session per account), replace the development identity policy
   behind a server switch (development login stays for local and training runs).
4. **Launcher:** device link UI and ticket fetch on macOS, Windows and Linux.
5. **Deployment:** homelab manifests (CNPG database, HTTPRoute, secrets), Google OAuth client.

## Goals, non-goals, anti-goals

- Goals: the three above; Google as the only identity provider at first; minimal personal
  data (Google subject ID, email for contact and operator lookup).
- Non-goals: a web admin UI (CLI first), more identity providers yet (the identity table
  keeps a provider column so Discord etc. can follow), name changes, account merging.
- Anti-goals: shipping Google tokens to the game or Medius; Medius depending on the website
  being up; a freed name silently inheriting someone else's characters.

## Decisions

| Decision | Choice | Why |
| --- | --- | --- |
| Where | `portal/` component in this repository | The ticket format, its signer and Medius's verifier change together; same release and chart tooling. |
| Stack | Rust (axum, sqlx, askama), server-rendered HTML, no JavaScript | Matches `gameplay/`; small attack surface. |
| Database | PostgreSQL | The homelab already runs CloudNativePG with offsite backups; account identity is precious. |
| Identity | Google OpenID Connect, authorization code flow with PKCE, state and nonce; ID token verified (RS256, issuer, audience, expiry, nonce) | Standard; endpoints configurable so tests run a fake provider. |
| Account key | Google `sub`, never email | Emails change and can be reassigned. |
| Names | 3–15 letters, digits, `-`, `_`, starting with a letter (the game's rule); unique ignoring case; case kept for display | Medius keys characters by lowercased name. |
| Freed names | Deleting an account leaves its name reserved | Characters in Medius are keyed by name; a new owner must not inherit them. |
| Existing names | Operator imports them as reserved and assigns them to their owners | Players who already have characters keep them; nobody else can claim them first. |
| Launcher linking | Device authorization flow (RFC 8628 shape): code shown in launcher, entered at `/link` | Works on all three platforms, including headless Linux; no local web server. |
| Device token | 256-bit random, stored as SHA-256, revocable on the account page | A leaked database does not leak usable tokens. |
| Ticket | Custom binary, ECDSA P-256 / SHA-256, r‖s signature, key ID for rotation, 10-minute default lifetime, 128-bit nonce | Small enough for the game's ticket slot; .NET verifies P1363 signatures natively. |
| Bans | Portal refuses tickets to banned accounts; Medius-side ban list in phase 3 | Short ticket lifetime bounds the window. |
| Sessions | Server-side, `__Host-` cookie, HttpOnly, Secure, SameSite=Lax, CSRF token on every form | Standard. |
| Admin | `mag-portal admin …` subcommands against the database, run with `kubectl exec` | Same pattern as `mag-admin`. |

## Security notes

- The game connection is not encrypted, so a ticket can be copied off the network. Short
  lifetime plus nonce use limits (phase 3: 3 uses per service, never a second live session for
  the account) bound replay to a few sign-ins within minutes; the game itself reuses one ticket.
- The audience field names the game server, so a ticket for one server is useless on another
  that trusts the same portal key.
- Device codes expire after 10 minutes and poll no faster than every 5 seconds.
- Every change to who holds or reserves a name takes a per-name PostgreSQL advisory lock, so
  a claim cannot slip in while a deletion reserves the name.
- Banned accounts cannot delete themselves; deleting and signing in again would shed the ban.
- Unauthenticated row-creating endpoints (link start, sign-in start) are rate limited per
  client address (per /64 for IPv6), taken from a trusted proxy header when configured.
- Known limitation: one sign-in flow per browser at a time (a second tab restarts it).

## Left to the builder

Page wording and styling, exact crate versions, cleanup intervals, log fields.
