# Changelog

## [0.4.0](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.3.0...gameplay-v0.4.0) (2026-11-04)


### Features

* bundle deployed server changelogs into in-game NEWS ([#42](https://github.com/tekulvw/mag-recomp-server/issues/42)) ([afb360e](https://github.com/tekulvw/mag-recomp-server/commit/afb360edf93156ad9f3c6842794cfab534ce3ace))
* **gameplay:** add Acquisition world queries and respawn transactions ([#46](https://github.com/tekulvw/mag-recomp-server/issues/46)) ([510772d](https://github.com/tekulvw/mag-recomp-server/commit/510772d1f1d30faaafaac2f12f5a74d05e9f3de8))
* **gameplay:** add native-bound Acquisition effect primitives ([#43](https://github.com/tekulvw/mag-recomp-server/issues/43)) ([fac38a1](https://github.com/tekulvw/mag-recomp-server/commit/fac38a1ab1403656811e18764479ac1d7095310e))
* **gameplay:** add owned Acquisition dynamic collision coverage ([#45](https://github.com/tekulvw/mag-recomp-server/issues/45)) ([8d2558a](https://github.com/tekulvw/mag-recomp-server/commit/8d2558a4661bf4f75bea00881f7cd0057670a46b))
* **gameplay:** add qualified retail ground vehicle creation ([#36](https://github.com/tekulvw/mag-recomp-server/issues/36)) ([42efaa1](https://github.com/tekulvw/mag-recomp-server/commit/42efaa1be8e68b455d1245c3e04e3b1b749f881b))
* **gameplay:** add typed human incapacitation and revive ([a79375e](https://github.com/tekulvw/mag-recomp-server/commit/a79375ec761de8f188e988f9aa902c01291a5f6b))
* **gameplay:** admit bounded retail Acquisition matches ([#35](https://github.com/tekulvw/mag-recomp-server/issues/35)) ([e311237](https://github.com/tekulvw/mag-recomp-server/commit/e3112374f797689bcb2976c2f05db7242beb44ad))
* **gameplay:** apply selected-match friendly damage rules ([226e596](https://github.com/tekulvw/mag-recomp-server/commit/226e59609d16cde213e58d5677afd9a8d7920420))
* **gameplay:** load qualified Acquisition objective catalogs ([#40](https://github.com/tekulvw/mag-recomp-server/issues/40)) ([0ef5eff](https://github.com/tekulvw/mag-recomp-server/commit/0ef5eff7f20b63112675b7f6d9d8d6eb1b0a9353))
* **gameplay:** persist committed in-match kit preferences ([a719eb3](https://github.com/tekulvw/mag-recomp-server/commit/a719eb3f48f09cc2bdab9f72c473ab32faa47ff5))
* **gameplay:** plan external Acquisition objective control ([#39](https://github.com/tekulvw/mag-recomp-server/issues/39)) ([cab8477](https://github.com/tekulvw/mag-recomp-server/commit/cab847713103bd8984600cf462d4e8ea6df52c09))
* **gameplay:** refresh active saved armory kits ([f617952](https://github.com/tekulvw/mag-recomp-server/commit/f6179524cde6dd4e6682e52c944f96766938c9a7))
* **gameplay:** require configurable ready-human team quorum ([e9e434c](https://github.com/tekulvw/mag-recomp-server/commit/e9e434c0a7f2635452c30292fcac298628eb8729))
* **gameplay:** stage atomic Acquisition callbacks ([#41](https://github.com/tekulvw/mag-recomp-server/issues/41)) ([a731497](https://github.com/tekulvw/mag-recomp-server/commit/a731497b8a9e54911fdd75b23135aa36cada48d2))
* **gameplay:** support Acquisition APC passenger insertion ([#38](https://github.com/tekulvw/mag-recomp-server/issues/38)) ([2998947](https://github.com/tekulvw/mag-recomp-server/commit/299894735502a20f6e5be434e7a137fbaaa846c6))
* **matchmaking:** form faction-aware matches from human queues ([#37](https://github.com/tekulvw/mag-recomp-server/issues/37)) ([fda3b37](https://github.com/tekulvw/mag-recomp-server/commit/fda3b3738e78d3ae2a6f718fc3bf7eea1c7b71d7))


### Bug Fixes

* **gameplay:** accept authenticated TCP revive requests ([532cd67](https://github.com/tekulvw/mag-recomp-server/commit/532cd67dd2ea3c70120d345b456ca1b0bc319e9b))
* **gameplay:** apply owned modifiers to falling damage ([#33](https://github.com/tekulvw/mag-recomp-server/issues/33)) ([4503039](https://github.com/tekulvw/mag-recomp-server/commit/4503039b119d5902c1fab57b4bc86fc4271d1375))
* **gameplay:** award human kills per initial collapse ([b2cd415](https://github.com/tekulvw/mag-recomp-server/commit/b2cd415b89032c297712f638e7c555753407d451))
* **gameplay:** award queued grenade kills against registry bots ([39b1682](https://github.com/tekulvw/mag-recomp-server/commit/39b1682b81b69a57f453bd870fe0ed451eb069d0))
* **gameplay:** bind Alpha mounting to configured match team ([#32](https://github.com/tekulvw/mag-recomp-server/issues/32)) ([6215607](https://github.com/tekulvw/mag-recomp-server/commit/62156074f5497ea5b72faa70672b1aee9564b419))
* **gameplay:** deny friendly bot firearm kill XP ([11660e4](https://github.com/tekulvw/mag-recomp-server/commit/11660e4165a1ca080c758dc4cd4e002e72c23549))
* **gameplay:** resolve bunker relationships by match team ([#31](https://github.com/tekulvw/mag-recomp-server/issues/31)) ([4ea2d82](https://github.com/tekulvw/mag-recomp-server/commit/4ea2d826f21f561b997653b95ebdc6bb4c2b6d56))
* **gameplay:** retain the current life equipped kit ([b11ae01](https://github.com/tekulvw/mag-recomp-server/commit/b11ae0187a7464925e047d53fa668a5860934495))
* **gameplay:** use actual teams for healing credit ([17388f1](https://github.com/tekulvw/mag-recomp-server/commit/17388f1f326e23a6317c174634f633f3e2d54944))

## [0.3.0](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.2.0...gameplay-v0.3.0) (2026-10-31)


### Features

* **gameplay:** apply authored bot recoil and continue long objective routes ([#14](https://github.com/tekulvw/mag-recomp-server/issues/14)) ([f4dc9eb](https://github.com/tekulvw/mag-recomp-server/commit/f4dc9ebc941016c01adc9e25e724ff5d2fe4e49d))
* **gameplay:** load external bot identities ([#13](https://github.com/tekulvw/mag-recomp-server/issues/13)) ([8b7c003](https://github.com/tekulvw/mag-recomp-server/commit/8b7c0030232a99a79e132990b31b457d16fadda9))


### Bug Fixes

* **gameplay:** advance human spawns on objective completion ([#15](https://github.com/tekulvw/mag-recomp-server/issues/15)) ([d520d83](https://github.com/tekulvw/mag-recomp-server/commit/d520d83dff032a02d0ba860b86dda9359d36564d))

## [0.2.0](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.1.1...gameplay-v0.2.0) (2026-10-31)


### Features

* use explicit retail catalogs for gameplay and deployment ([#11](https://github.com/tekulvw/mag-recomp-server/issues/11)) ([53495c9](https://github.com/tekulvw/mag-recomp-server/commit/53495c9841891a90bf3535c5770f2619f20860fa))

## [0.1.1](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.1.0...gameplay-v0.1.1) (2026-10-31)


### Bug Fixes

* **gameplay:** release disconnected world subscriptions ([#8](https://github.com/tekulvw/mag-recomp-server/issues/8)) ([67ac36a](https://github.com/tekulvw/mag-recomp-server/commit/67ac36a27daaeead2a08267dbb7841f9d92fbbe3))
