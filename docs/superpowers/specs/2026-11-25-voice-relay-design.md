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
- No recording. Moderation is server-side voice mutes and bans per account ("Moderation").
- In-match only. There is still no party voice world (see `docs/configured-world-registry.md`).
- Off by default behind an explicit switch; mag-test first, then live, where the player's own client
  setting keeps it opt-in.

## What the game expects from the server

- A client sends each voice packet to the DME addressed to a relay group, in its own SCE-RT message
  type (53 in the lab capture; U1 is settled for the client-to-server direction). The packet carries the
  group (one byte), a media type (one byte), the data size (two bytes), a sequence number (two bytes)
  that counts per talker and group, and at most 248 data bytes; "Lab findings" below gives the observed
  layout. The server delivers it to every other client that listens to that group, marked with the
  sender's DME client index ("Listener form" below settles U10). The game server never reads or
  changes the audio bytes, and the receiver tracks sequence per sender.
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
  set; the talker's byte rate is within a budget of twice the nominal voice byte rate, with one second
  of burst (excess dropped). The budget counts bytes, not packets, and there is no per-packet size limit
  below the game's own 248 data bytes, because a game that falls behind packs several blocks into one
  packet (up to six 37-byte blocks, 222 data bytes, seen in the lab) and the catch-up must pass. A
  packet failing a check is dropped and counted, never forwarded.
- **Proximity copies.** A packet to a proximity group is allowed only on the talker's own group
  (83 + its DME client index); any other proximity group is refused as `not_allowed_group`. While
  gameplay lists no proximity group in the talker's transmit set (the default, since nothing produces
  one yet) the copy is dropped and counted as `proximity_off`, so half of all voice traffic shows up
  there on a healthy server. Once listed, it is relayed to the listeners gameplay put in that group,
  enemies included, with its own byte budget beside the channel copy's.
- **Fan-out.** Deliver to every other bound client whose listen set contains the group, in the
  listener form below, stamped with the talker's index, over the recipient's UDP channel when it has
  one and TCP otherwise. Never to the talker, never across sides except on a proximity group, never
  to party groups. One message is built per packet and queued once per recipient.
- **Malformed sends.** While the relay is on, a group send that is not one well-formed voice packet
  (one addressed group, message type 47, a length that matches) is dropped and counted as `malformed`.
- **State.** The relay table is the latest `voice_groups` revision. Until a world has received one,
  nothing is relayed. A client that disconnects leaves every set at once; its next revision confirms it.
- **Counters.** Per world: packets and bytes received, relayed and dropped by reason. A `Voice relay:`
  line logs them at most once a minute, and only after new packets; no per-packet logging outside the
  diagnostic switch.

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
  buffer drops the surplus.
- **Rate settled (14:23 to 14:26 UTC, with the per-group receipts).** The client handed the game 2,125
  blocks. The DME received 2,092 squad packets carrying 78,699 data bytes (2,127 blocks of 37) and
  2,088 proximity packets carrying 78,736 (2,128 blocks), with no sequence gaps, no steps back and no
  length mismatches. The largest packets held 222 and 185 data bytes (six and five blocks); 26 and 32
  packets held more than one block. So every block arrives: when the game's send loop runs slower than
  the block rate it packs the waiting blocks into one packet. The packet count understates the voice;
  the byte count is exact. The relay forwards such packets as they are.

## Listener form (2026-11-26, static reading of the client's transport library)

No listener capture exists yet, so this comes from reading how the game's own transport library
handles messages, in both the prototype and the retail build; the two agree.

- **The uplink address block.** A group send (transport message 53, or 54 for the same send with
  the sender's other delivery flag) opens with an address block, not with the voice packet. Its first
  byte counts the group-mask bytes in its low seven bits; its top bit says a two-byte little-endian
  base group follows. Then come the mask bytes, where bit b of mask byte i names group base + 8i + b,
  and after them the application data. The lab's `81 GG 00 01` is one mask byte with a base, the base
  group `GG 00` and the mask `01`. A send to group 0 has no base: `01 01`. So the "14-byte header"
  is a 4-byte address block plus a 10-byte voice header that opens with the message type 47, and
  groups above 255 fit in the address block (U11) even though the voice header keeps one byte.
- **What a client reads.** For every app-family message from the server (client app single,
  broadcast, list, their QoS variants and the group sends 53 and 54) the library reads a two-byte
  little-endian source client index and hands the rest to the network layer as application data
  from that client. Messages the server itself originates carry no index and read as "no client",
  which the voice handler refuses (it needs an index below 512).
- **So the relay sends** the talker's DME client index, then the application data exactly as it
  arrived (the message from type 47 on: group, media type, length, sequence and data unchanged). The
  default form is a client app single with the talker as its source, which is how the DME forwards
  other peer traffic. The talker's index is the only stamp; the sequence is never renumbered.
- **Confirmed by hearing.** On mag-test, with the relay on and `MAG_VOICE_RELAY_TIER=all`,
  a listener heard a talker in this form. It is now the only form: the lab switch
  `MAG_VOICE_RELAY_FORM` and its alternatives (`app53`, `from_b2`, `whole`, `from_b6`) are removed. A
  value still set is ignored, with a warning in the Medius log, so an old deployment keeps starting.
- **Checking it from a listener.** On the receiving client (voice on, `MAG_VOICE_TRACE` set), a form the
  game accepts shows `connect` events beyond the three at start-up (a talker slot's voice port joined to
  its PCM port) and `write` events on that talker's voice port, then a summary with `received_blocks`
  rising and `frames_decoded` and `samples_played` (tone mode) or `pattern_ok` (pattern mode), with
  `sequence_gaps` and `skipped_bytes` near zero. A form it ignores shows only the start-up `connect`s
  and no `write`. The DME's own `Voice relay:` counters line confirms the packets left the server.

## Moderation

Server-side voice moderation is per account: a **voice mute** or a **voice ban**. Neither retail nor
this design gives listeners anything to see or do about it. Retail's own mutes are client-side (the
self-mute and the per-channel listen mute, "Lab findings"), and nothing on the listener side tells a
mute from a ban. So the relay treats the two alike; the kind records the operator's intent: a mute is a
cool-off, a ban a standing refusal.

- **Effect.** While a sanction is active the relay drops every voice packet the account sends, its
  channel copy and its proximity copy, and counts each as `muted`. The player still hears everyone
  else and is not told. A ban refuses the account's voice for its whole duration in the same way.
- **Duration.** Either kind may end at a time (`--for 30m`, `12h`, `7d`, `2w`; from one minute to ten
  years) or last until lifted. Imposing a kind again replaces that kind's sanction; a mute and a ban
  can stand together, and the relay reports the ban.
- **Storage.** In the character store beside the characters: PostgreSQL table `mag.voice_sanctions`
  (migration 0002; account, kind, reason, operator, created and expiry times, at most one row per
  account and kind, deleted with its character) and the JSON store's `voiceSanctions` list. A lifted
  sanction leaves the table; an expired one stays until replaced or lifted, and is ignored. Import,
  export, verify and the contents digest carry sanctions like any other record.
- **Journal.** Every applied change (`voice-mute`, `voice-ban`, `voice-unmute`, `voice-unban`) is
  journalled with the sanction before it, the sanction after it, the operator and the reason: in
  `mag.admin_changes` in the same transaction, or, for the JSON store, as a copy of the file before the
  change in `.maintenance-backups/`.
- **Caching.** Main Medius (with the local DME and `MAG_VOICE_RELAY=1`) loads the active sanctions into
  memory at start, reloads them every 5 seconds and at once after the admin API applies a change. The
  relay checks the talker's account against that snapshot for each packet: one lookup, never a
  database query. Expiry is checked at each lookup, so a sanction ends on time without a reload. If a
  reload fails, the last snapshot stays and Medius logs it once; with no snapshot at all, nobody is
  silenced. The log line names counts only, never accounts or reasons.
- **Tools.** `mag-admin voice list`, `voice mute|ban <account> [--for DURATION] [--reason TEXT]
  [--operator NAME] [--apply]` and `voice unmute|unban <account> [--reason TEXT] [--operator NAME]
  [--apply]`, dry runs without `--apply`; and the admin API's `GET /admin/v1/voice` and
  `POST /admin/v1/voice/{account}/{mute|ban|unmute|unban}` under its signed authentication, the actor
  being the operator. See [admin-cli.md](../../admin-cli.md#voice-moderation). A sanction needs an account
  with a character, and works whether or not the account is online.

## Switches

| Switch | Where | Default | Effect |
| --- | --- | --- | --- |
| `MAG_VOICE_RELAY` | medius (DME) environment | off | relay voice; with it off the DME never parses group sends and they keep today's path (logged as unhandled) |
| `MAG_VOICE_RELAY_TIER` | medius (DME) environment | `all` | `all` relays every static tier, the retail channels; `squad` relays squad groups only, for a staged rollout. Any other value stops Medius at start-up |
| `voice_routing` | gameplay configuration | off | decode `0x0259` into the channel record and publish `voice_groups` |
| `MAG_VOICE_DIAG` | medius and gameplay environment, lab only | off | per-packet receipts (below) |

With every switch off the server behaves as today, except that `0x0259` stays logged as unknown.
The default tier was `squad` until the hearing test passed; it is now `all`, since retail serves every
static tier and the binding decision is retail-exact channels. The counters line names the tier
(`tier=all`).

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
4. Platoon and Company chat, both broadcasts, the transmit checks for every tier, moderation commands
   (account voice mutes and bans, "Moderation").
5. mag-test soak, then live with the relay on and clients opt-in.

## Synthetic tests

Membership and set rules for every role and for players without a squad or leaving; revisions on
placement and election changes; the `voice_groups` frame round trip; relay checks (disabled, unbound,
muted, banned, wrong group, over budget) with counters; fan-out targets for each tier (never the talker,
never the other side, never a party group); proximity copies (only the talker's own group, dropped as
`proximity_off` until gameplay lists it, then to nearby listeners of both sides with their own budget);
the voice header parser; the diagnostic stream counters; the 8-group cap; and that voice never reaches the gameplay
event bridge.
