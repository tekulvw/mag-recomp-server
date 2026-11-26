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

- A client sends each voice packet to the DME addressed to a relay group, in its own SCE-RT message
  type (53 in the lab capture; U1 is settled for the client-to-server direction). The packet carries the
  group (one byte), a media type (one byte), the data size (two bytes), a sequence number (two bytes)
  that counts per talker and group, and at most 248 data bytes; "Lab findings" below gives the observed
  layout. The server delivers it to every other client that listens to that group, marked with the
  sender's DME client index (U10, the server-to-client form, is still open). The game server never
  reads or changes the audio bytes, and the receiver tracks sequence per sender.
- **Every frame is sent twice:** once to the group of the talker's selected channel and once to the
  talker's own proximity group (83 + its DME client index), whether or not proximity chat is on. With
  no selected channel (channel 0) only the proximity copy is sent.
- Game message `0x0259` carries one byte, the talk channel. The client sends it when its voice menu
  opens and on each change a leader makes. Gameplay records it per player.
- Each client listens to at most 8 groups. The listen sets follow from the side, platoon, squad and
  leadership role the server already holds.

## Channels, groups and roles

Channels, by the game's wire value: 1 Squad chat, 2 Platoon chat, 3 Company chat, 4 Platoon broadcast,
5 Team broadcast, 6 Proximity (later), 7 Party (out of scope), 0 none.

Relay groups, numbered as the client computes them from its side and world squad index (0 to 31; a
platoon is four consecutive squads): team group = side (0, 1); company group = 2 + side; platoon group =
4 + platoon; platoon-broadcast group = 12 + platoon; squad group = 20 + squad index. 52 to 82 are party
groups, which the relay never serves. Retail also has 256 proximity groups (83 to 338), one per DME
client, which talks on 83 + its index; see "Proximity room". Retail's "no group" value is 339, which the
client never sends.

Who hears each channel (the retail delivery rule): Squad chat, the talker's squad; Platoon chat, the
platoon leader and the platoon's four squad leaders; Company chat, the OIC and the platoon leaders;
Platoon broadcast, the whole platoon; Team broadcast, the whole side. So each player's listen set is:

| Role | Listens to |
| --- | --- |
| Member | own squad, own platoon broadcast, own team |
| Squad leader | as member, plus own platoon |
| Platoon leader | as member, plus own platoon and own company |
| OIC | as member, plus own company |

The transmit set follows the channels the game's menu offers each role:

| Role | May transmit to |
| --- | --- |
| Member | own squad |
| Squad leader | own squad, own platoon |
| Platoon leader, worlds with more than one platoon a side | own squad, own platoon, own company, own platoon broadcast |
| Platoon leader, one platoon a side (64-player modes, no OIC) | own squad, own platoon, own team |
| OIC | own squad, own platoon, own company, own team |

That is at most five listen groups per player, within the client's limit of 8. A group's members are
exactly the players whose listen set contains it. A player without a squad transmits to nothing and
listens to the team group only, until placed. Retail's server did not check the stored channel; we do
check the transmit set, which only refuses what the menu never offers.

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
  per-player sets. Retail gives every player a proximity group of their own; proximity will put each
  listener near a talker, enemies included, into that talker's proximity group, recomputed from
  positions on a slow cadence (a few times a second) and published the same way. As in retail, a
  listener who already hears the talker through the talker's recorded channel (`0x0259`) is left out of
  that talker's proximity group, so nobody hears a frame twice; that is the only use the channel record
  has on the server. The talker's own proximity group joins its transmit set only while proximity is on. Proximity groups pass
  the 8-group listen limit quickly in a crowd, so the nearest talkers win. Nothing in the frame format
  assumes the static tiers are the only source.

## DME relay

- **Recognising voice.** The DME identifies a voice packet by its transport message and header (U1)
  before the local-gameplay event path, so voice never reaches the gameplay bridge.
- **Checks, in order:** voice relay enabled for this world; the talker is a bound, admitted client; the
  talker's account is not voice-muted or voice-banned; the packet's group is in the talker's transmit
  set; the talker's packet rate is within a budget of twice the nominal voice byte rate (excess
  dropped). A packet failing a check is dropped and counted, never forwarded.
- **Proximity copies.** A packet to a proximity group is allowed only on the talker's own group
  (83 + its DME client index); any other proximity group is refused as `not_allowed_group`. While
  gameplay lists no proximity group in the talker's transmit set (the default, since nothing produces
  one yet) the copy is dropped and counted as `proximity_off`, so half of all voice traffic shows up
  there on a healthy server. Once listed, it is relayed to the listeners gameplay put in that group,
  enemies included, with its own byte budget beside the channel copy's.
- **Fan-out.** Deliver to every other bound client whose listen set contains the group, in the
  server-to-client form the game expects (U1), stamped with the talker's index (U10), over the
  recipient's UDP channel when it has one and TCP otherwise. Never to the talker, never across sides
  except on a proximity group, never to party groups.
- **State.** The relay table is the latest `voice_groups` revision. Until a world has received one,
  nothing is relayed. A client that disconnects leaves every set at once; its next revision confirms it.
- **Counters.** Per world: packets and bytes received, relayed and dropped by reason. They appear in the
  existing periodic world log line; no per-packet logging outside the diagnostic switch.

## Lab findings (2026-11-26)

One retail client with the lab pattern mode, on mag-test (gameplay 0.16.0, Medius 0.12.0,
`MAG_VOICE_DIAG=1`, relay off), playing Sabotage on the defending side. The receipts stay in the private
evidence store; these are the conclusions.

- **Wire layout.** Each voice packet is a 14-byte header and the data: byte 1 and byte 6 both carry the
  group, bytes 4-5 the voice message kind (0x00 then 0x2F, 47), byte 7 the media type, bytes 8-9 the
  data length and bytes 10-11 the sequence, both little-endian. The lab client's frames were 37-byte blocks (51-byte
  packets), about 12.5 a second per group. Bytes 0, 2-3 and 12-13 are not interpreted. The group field
  is one byte, so how retail carried proximity groups above 255 (client indices above 172) is still
  open (U11, new and server-only); it matters only for worlds above 172 clients.
- **Group 83 is the talker's proximity group,** not an "all" or "invalid" group: the client was DME
  client 0, and the game always adds a copy for 83 + its own client index. Retail's game server chose
  who listens to a proximity group from distance, a few clients per frame in turn, enemies included by
  default, and skipped a listener who already hears the talker through the talker's selected channel,
  so nobody gets both copies. The DME itself only relayed by group membership: with proximity off no
  one joins a proximity group and its copies reach nobody. The retail configuration names a proximity
  switch, its radius, whether enemies are included, and whether the squad, platoon or team are always
  included; none of these has to exist on our side until proximity is built.
- **Group mapping verified.** The player was elected squad leader of world squad 16 (first platoon,
  first squad of side 1). The channel copies went to 36 (squad, 20 + 16) and, after the leader rotated
  to Platoon chat, to 8 (platoon, 4 + 16 / 4). Both match the formulas above.
- **0x0259 timing.** It is sent only on presses: 1 when the voice menu opens or a leader rotates to
  Squad, 2 on a rotation to Platoon. A player who never opens the menu stays on Squad.
- **Before spawn.** Squad traffic starts in pre-battle, before the first spawn, as soon as the client
  is connected and placed. Gameplay places a human in a squad when its initial squad snapshot is sent,
  which is before pre-battle, so the relay serves pre-battle voice with no extra rule. Retail's client
  records from the moment it joins the game server.
- **Mutes.** Self-mute stops the client sending: the DME sees no traffic. The per-channel listen mute
  is client-only. Neither needs server support; server-side account mutes and bans drop both copies.
- **Rate in match.** In the second pass (10:19 to 10:24 UTC) the DME counted about 83 packets per 10 s
  per group in match against 125 before spawn, both copies alike, while the client handed the game
  every block (12.5 a second, none dropped). The game read the voice port about 57 times a second
  before spawn and about 33 in match; the packet rate per group tracked a quarter of the read rate,
  capped by the block rate. The DME showed no decode errors, its receive queue has no cap, and both
  groups lost exactly the same share, which loss in transit would not produce. So the shortfall is on
  the sending side: either the game packs more than one block into a packet (harmless) or its own
  buffer drops the surplus. The diagnostic receipts now tell the two apart (below).

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
hex and the time, for the first 20 messages of each kind and then one in every 100 for as long as it
lasts. Every ten seconds a counts line per message kind follows, and one per talker and voice group
with the packets, data bytes, smallest and largest data length, packets holding more than one 37-byte
block, sequence gaps (packets the DME never saw), sequence steps back (repeats or a restarted
client) and messages with the voice kind whose length does not match one packet. A message kind that
first appears after 512 kinds are in use (messages with random leading bytes fill the table in a
long-lived process) is still counted and sampled, as "lead=other" under its message type, and a kind
with no message for a whole ten-second interval leaves the table, so a long lab session never goes
blind; in gameplay, each `0x0259` with its body bytes, the player and their squad, platoon and
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
never the other side, never a party group); proximity copies (only the talker's own group, dropped as
`proximity_off` until gameplay lists it, then to nearby listeners of both sides with their own budget);
the voice header parser; the diagnostic stream counters; the 8-group cap; and that voice never reaches the gameplay
event bridge.
