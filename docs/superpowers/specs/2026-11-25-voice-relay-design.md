# In-match voice relay

The user approved in-match voice chat on November 25, 2026, as lower priority than live fixes. The
client half (the runtime voice service, its framing, push to talk and privacy) is specified in
mag-recomp's `docs/superpowers/specs/2026-11-25-voice-comms-design.md`. This document specifies the
server half in our own words: who may talk and listen on which channel, how gameplay tells the DME, and
how the DME relays audio. Points still to be confirmed by the lab capture are marked **(U*n*)**, with the
same numbers as the client document.

## Decisions that bind the server

- Retail channels exactly: Squad, Platoon and Company chat, Platoon broadcast and Team broadcast, with
  the retail role gating.
- No proximity yet, but the routing keeps room for it; when it comes, enemies can hear proximity speech.
- No recording. Moderation is server-side voice mutes and bans per account.
- In-match only. There is still no party voice world (see `docs/configured-world-registry.md`).
- Off by default behind an explicit switch; mag-test first, then live, where the player's own client
  setting keeps it opt-in.

## What the game expects from the server

- A client sends each voice packet to the DME tagged with a relay group number (U1). The server delivers
  it to every other client that listens to that group, marked with the talker's client index (U10). The
  game server never reads or changes the audio bytes.
- When a player changes talk channel, the client sends game message `0x0259` naming the channel (U7).
  Gameplay records it per player.
- Each client listens to at most 8 groups. The listen sets follow from the side, platoon, squad and
  leadership role the server already holds.

## Channels, groups and roles

Channels, by the game's wire value: 1 Squad chat, 2 Platoon chat, 3 Company chat, 4 Platoon broadcast,
5 Team broadcast, 6 Proximity (later), 7 Party (out of scope), 0 none.

Relay groups (83 per world; exact numbers U8): per side one team group and one company group; per
platoon a platoon group and a platoon-broadcast group; per squad a squad group; and party groups, which
the relay never serves.

Working rule for roles (U6, to be confirmed before the leader tiers ship):

| Role | Listens to | May transmit to |
| --- | --- | --- |
| Member | own squad, own platoon broadcast, own team | own squad |
| Squad leader | as member, plus own platoon | as member, plus own platoon |
| Platoon leader | as squad leader, plus own company | own squad (if in one), own platoon, own company, own platoon broadcast |
| OIC | as member, plus own company | own squad (if in one), own company, own team |

That is at most five groups per player. A group's members are exactly the players whose listen set
contains it. A player with no squad listens to the team and company groups their role gives them and
transmits to nothing until placed.

## Gameplay

- **Channel record.** Decode message `0x0259` (today it is logged as unknown) into the player's talk
  channel and keep it per human. It is advisory: it never widens what a player may transmit to.
- **Membership.** For every human in a world, compute a voice membership from state gameplay already
  owns: side, platoon, squad and held leadership role. From it derive the listen set and the transmit
  set by the table above.
- **Publication.** Send the memberships to Horizon on the world stream as a `voice_groups` frame: a
  revision number and, per human DME client, its listen groups and transmit groups. Send a full frame
  when voice is first enabled and after a stream resume, and a new revision whenever any membership
  changes (squad placement, leadership election or hand-over, a player leaving). Never per tick.
- **No audio.** Gameplay never receives voice packets; the DME keeps them off the event bridge, which
  would otherwise carry one request per packet.
- **Proximity room.** The routing is one function from a world's memberships (and, later, positions) to
  per-player sets. Proximity will add a per-talker audience computed from positions on a slow cadence
  (a few times a second), enemies included, published the same way as an audience list for the
  proximity channel. Nothing in the frame format assumes the static tiers are the only source.

## DME relay

- **Recognising voice.** The DME identifies a voice packet by its transport message and header (U1)
  before the local-gameplay event path, so voice never reaches the gameplay bridge.
- **Checks, in order:** voice relay enabled for this world; the talker is a bound, admitted client; the
  talker's account is not voice-muted or voice-banned; the packet's group is in the talker's transmit
  set; the talker's packet rate is within a budget of twice the nominal voice byte rate (excess
  dropped). A packet failing a check is dropped and counted, never forwarded.
- **Fan-out.** Deliver to every other bound client whose listen set contains the group, in the
  server-to-client form the game expects (U1), stamped with the talker's index (U10), over the
  recipient's UDP channel when it has one and TCP otherwise. Never to the talker, never across sides
  (until proximity), never to party groups.
- **State.** The relay table is the latest `voice_groups` revision. Until a world has received one,
  nothing is relayed. A client that disconnects leaves every set at once; its next revision confirms it.
- **Counters.** Per world: packets and bytes received, relayed and dropped by reason. They appear in the
  existing periodic world log line; no per-packet logging outside the diagnostic switch.

## Moderation

- **Voice mute** (an account's voice is not relayed for a time) and **voice ban** (until lifted), stored
  per account with the other account flags. The relay drops that account's packets silently; the
  player still hears others.
- Managed with `mag-admin` (new `voice mute <account> <minutes>`, `voice ban <account>`,
  `voice unban <account>`, dry run by default like its other writes). Changes apply to connected players
  within a few seconds.
- The game's own per-player mute stays a client feature and needs nothing from the server.

## Switches

| Switch | Where | Default | Effect |
| --- | --- | --- | --- |
| `MAG_VOICE_RELAY` | medius (DME) environment, chart value | off | relay voice; with it off voice packets are dropped and counted as `relay_off` |
| `voice_routing` | gameplay configuration | off | decode `0x0259` into the channel record and publish `voice_groups` |
| `MAG_VOICE_DIAG` | medius and gameplay environment, lab only | off | per-packet receipts (below) |

With every switch off the server behaves as today, except that `0x0259` stays logged as unknown.

## Diagnostics for the lab capture

`MAG_VOICE_DIAG=1` writes per-packet receipts for the lab: in the DME, for each client message that is
not ordinary gameplay traffic, the transport message type, total length, the first 32 header bytes in
hex and the time; in gameplay, each `0x0259` with its body bytes, the player and their squad, platoon and
role at that moment. The receipts are runtime captures: they stay in the private evidence stores, never
in Git, and the switch is never set on live. With pattern payloads from the client's lab mode the
receipts contain no speech.

## Privacy

- The relay forwards audio in the clear over the game's own transport; nothing stores it, and no log,
  metric or report contains voice bytes outside the lab diagnostic above.
- Live keeps voice opt-in per player: a client sends nothing unless its player turned voice on.

## Stages

1. This document, with the client specification.
2. Diagnostic receipts (`MAG_VOICE_DIAG`), off by default, for the lab capture on mag-test.
3. Squad only: the channel record, memberships, `voice_groups` and the DME relay for squad groups.
   Opt-in on mag-test.
4. Platoon and Company chat, both broadcasts, the transmit checks for every tier, moderation commands.
5. mag-test soak, then live with the relay on and clients opt-in.

## Synthetic tests

Membership and set rules for every role and for players without a squad or leaving; revisions on
placement and election changes; the `voice_groups` frame round trip; relay checks (disabled, unbound,
muted, banned, wrong group, over budget) with counters; fan-out targets for each tier (never the talker,
never the other side, never a party group); the 8-group cap; and that voice never reaches the gameplay
event bridge.
