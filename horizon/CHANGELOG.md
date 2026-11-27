# Changelog

## [0.14.1](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.14.0...medius-v0.14.1) (2026-11-27)


### Bug Fixes

* **medius:** make the character export a verified rollback copy ([#399](https://github.com/tekulvw/mag-recomp-server/issues/399)) ([f778af7](https://github.com/tekulvw/mag-recomp-server/commit/f778af7b354aee19cea34e89827f728747a6035f))

## [0.14.0](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.13.1...medius-v0.14.0) (2026-11-27)


### Features

* **medius:** add mag-admin character kill to unstick live players ([#390](https://github.com/tekulvw/mag-recomp-server/issues/390)) ([554fdb9](https://github.com/tekulvw/mag-recomp-server/commit/554fdb903e0fa9b9f131d4c1e01229fe01ba6220))
* **voice:** relay every static tier by default and keep only the app listener form ([#396](https://github.com/tekulvw/mag-recomp-server/issues/396)) ([49784f3](https://github.com/tekulvw/mag-recomp-server/commit/49784f34f7558c95bc9e6ec79af37677cc0da4c9))
* **voice:** relay group-addressed voice sends to listeners behind MAG_VOICE_RELAY ([#395](https://github.com/tekulvw/mag-recomp-server/issues/395)) ([ceb93eb](https://github.com/tekulvw/mag-recomp-server/commit/ceb93eb101376dd583251efffd0229b4dea7dce6))
* **voice:** server-side voice mutes and bans per account ([#397](https://github.com/tekulvw/mag-recomp-server/issues/397)) ([5cce443](https://github.com/tekulvw/mag-recomp-server/commit/5cce443b4edbf459d5c59abd11e36486476d28d1))

## [0.13.1](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.13.0...medius-v0.13.1) (2026-11-26)


### Bug Fixes

* **medius:** keep voice traffic visible in the DME lab receipts and parse its header ([#382](https://github.com/tekulvw/mag-recomp-server/issues/382)) ([e7f17b6](https://github.com/tekulvw/mag-recomp-server/commit/e7f17b6c025ac7173edf1b0f002082d6be107f70))

## [0.13.0](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.12.0...medius-v0.13.0) (2026-11-26)


### Features

* **voice:** treat proximity copies as proximity_off and keep sampling lab receipts ([#375](https://github.com/tekulvw/mag-recomp-server/issues/375)) ([971a386](https://github.com/tekulvw/mag-recomp-server/commit/971a3868bd98482e9ce8a61bfe95d2bfd33a0a90))


### Bug Fixes

* **medius:** report each world's effective human limit in the server status ([#371](https://github.com/tekulvw/mag-recomp-server/issues/371)) ([f972a92](https://github.com/tekulvw/mag-recomp-server/commit/f972a921d5854bba2a7bc4521f23437a98db24aa))

## [0.12.0](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.11.0...medius-v0.12.0) (2026-11-26)


### Features

* **medius:** make character-store import idempotent, with a dry run, counts and digests ([#365](https://github.com/tekulvw/mag-recomp-server/issues/365)) ([42ae55f](https://github.com/tekulvw/mag-recomp-server/commit/42ae55f37e9d2e3d22be32d1890c1fe39929c031))
* **medius:** report read-only server status on the admin API ([#361](https://github.com/tekulvw/mag-recomp-server/issues/361)) ([df62be3](https://github.com/tekulvw/mag-recomp-server/commit/df62be352eaf93bf2f3edfdad6fcbf2eac20eca8))
* **voice:** add lab diagnostic receipts for the voice capture ([#342](https://github.com/tekulvw/mag-recomp-server/issues/342)) ([e45091c](https://github.com/tekulvw/mag-recomp-server/commit/e45091c93c7566ab1fd1d226221bcbb34c150a8e))
* **voice:** publish voice group memberships and add the DME relay decision ([#345](https://github.com/tekulvw/mag-recomp-server/issues/345)) ([e120806](https://github.com/tekulvw/mag-recomp-server/commit/e1208068010b98746992c6582683d5baea7b4cf7))


### Bug Fixes

* **medius:** reconnect cleanly to a worker that outlived Medius ([#358](https://github.com/tekulvw/mag-recomp-server/issues/358)) ([dc9e94e](https://github.com/tekulvw/mag-recomp-server/commit/dc9e94e634e8ac3a6b48f3952800806cfd002268))
* **medius:** stop probing GSSAPI on character database connections ([#367](https://github.com/tekulvw/mag-recomp-server/issues/367)) ([0f4bdae](https://github.com/tekulvw/mag-recomp-server/commit/0f4bdaebf02271b2e7490a96eb1ccb8a52959972))

## [0.11.0](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.10.2...medius-v0.11.0) (2026-11-26)


### Features

* **medius:** keep characters in PostgreSQL behind MAG_CHARACTER_STORE ([#315](https://github.com/tekulvw/mag-recomp-server/issues/315)) ([2e79200](https://github.com/tekulvw/mag-recomp-server/commit/2e792009dd3571fe64d137f7421d985321f33d20))


### Performance Improvements

* give each recipient its own bridge reply allowance behind a worker flag ([#318](https://github.com/tekulvw/mag-recomp-server/issues/318)) ([d12d67f](https://github.com/tekulvw/mag-recomp-server/commit/d12d67f2da4afb8e52c8a73ca5f409341828c8b9))

## [0.10.2](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.10.1...medius-v0.10.2) (2026-11-26)


### Bug Fixes

* **medius:** compact settled reward history so the character store keeps accepting rewards ([#301](https://github.com/tekulvw/mag-recomp-server/issues/301)) ([26df337](https://github.com/tekulvw/mag-recomp-server/commit/26df337f1cc4f1571cdeece156644a8d104b342a))
* **medius:** expire abandoned open reward rows so they cannot fill the character store ([#305](https://github.com/tekulvw/mag-recomp-server/issues/305)) ([3219c3e](https://github.com/tekulvw/mag-recomp-server/commit/3219c3ec122f64b852e095c05eba12691d6cfd57))

## [0.10.1](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.10.0...medius-v0.10.1) (2026-11-25)


### Bug Fixes

* **medius:** give party admission the same bounded retries ([#296](https://github.com/tekulvw/mag-recomp-server/issues/296)) ([b8215ea](https://github.com/tekulvw/mag-recomp-server/commit/b8215eac1e08aaa19b5af14a8f939fb7969bcd4e))
* **medius:** retry gameplay admission the worker did not answer ([#293](https://github.com/tekulvw/mag-recomp-server/issues/293)) ([73ad20f](https://github.com/tekulvw/mag-recomp-server/commit/73ad20fa5db223bbc715459d0366f336ce8a73d3))

## [0.10.0](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.9.0...medius-v0.10.0) (2026-11-25)


### Features

* **medius:** admit humans against a human cap separate from world size ([#263](https://github.com/tekulvw/mag-recomp-server/issues/263)) ([0e853aa](https://github.com/tekulvw/mag-recomp-server/commit/0e853aa115f5975c9c6c6b9a18737cfbe9afe7e9))


### Bug Fixes

* **medius:** answer friend-card online and game-info queries ([#267](https://github.com/tekulvw/mag-recomp-server/issues/267)) ([e4297e8](https://github.com/tekulvw/mag-recomp-server/commit/e4297e816a3078d59000ac3e7423be47b64786d7))

## [0.9.0](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.8.3...medius-v0.9.0) (2026-11-24)


### Features

* **medius:** adopt a restarted registry worker without restarting Medius ([#255](https://github.com/tekulvw/mag-recomp-server/issues/255)) ([983b5b9](https://github.com/tekulvw/mag-recomp-server/commit/983b5b9681afe336ca78dbf0df5caa4903b8bfa7))
* **medius:** stop admitting players beyond the forwarded DME ports ([#253](https://github.com/tekulvw/mag-recomp-server/issues/253)) ([ff401f1](https://github.com/tekulvw/mag-recomp-server/commit/ff401f1353e3b8454b933d9a503a6d522f77e92d))


### Bug Fixes

* **medius:** answer the retail clan invitations request ([#257](https://github.com/tekulvw/mag-recomp-server/issues/257)) ([3b1e1b4](https://github.com/tekulvw/mag-recomp-server/commit/3b1e1b4866a1051354a51d8db17306c5b47611c1))

## [0.8.3](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.8.2...medius-v0.8.3) (2026-11-24)


### Bug Fixes

* **medius:** answer the launcher's account logout request ([#244](https://github.com/tekulvw/mag-recomp-server/issues/244)) ([278eeec](https://github.com/tekulvw/mag-recomp-server/commit/278eeec58adf7c4f9cae6952cbb98bc86a329798))
* **medius:** send a non-zero MLS plugin id and answer the retail clan list ([#248](https://github.com/tekulvw/mag-recomp-server/issues/248)) ([dc176b2](https://github.com/tekulvw/mag-recomp-server/commit/dc176b2240130ff22eb9c258e0087632d62a9f6b))

## [0.8.2](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.8.1...medius-v0.8.2) (2026-11-24)


### Bug Fixes

* **medius:** save the lobby squad-leader volunteer preference ([#239](https://github.com/tekulvw/mag-recomp-server/issues/239)) ([6f0ae32](https://github.com/tekulvw/mag-recomp-server/commit/6f0ae328c5daaedce77161ea57dba915d8f88372))

## [0.8.1](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.8.0...medius-v0.8.1) (2026-11-24)


### Bug Fixes

* **gameplay:** keep players in the match when a bullet kills a bot after the result ([#227](https://github.com/tekulvw/mag-recomp-server/issues/227)) ([9022218](https://github.com/tekulvw/mag-recomp-server/commit/9022218313ce1f1f00452535d0436f3d40011e39))
* **medius:** read retail party invite answers in client order and notify inviters ([#231](https://github.com/tekulvw/mag-recomp-server/issues/231)) ([fcebd2c](https://github.com/tekulvw/mag-recomp-server/commit/fcebd2c7821c81ced4dbbd1a6d243a6ca49e549b))

## [0.8.0](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.7.1...medius-v0.8.0) (2026-11-24)


### Features

* **chart:** run configured gameplay workers behind the world registry ([#214](https://github.com/tekulvw/mag-recomp-server/issues/214)) ([6fb825f](https://github.com/tekulvw/mag-recomp-server/commit/6fb825fc621f84a12ab41c88e3541fee5ce783d1))
* **gameplay:** fill match profiles with per-profile bots and start them with one human ([#213](https://github.com/tekulvw/mag-recomp-server/issues/213)) ([b840db8](https://github.com/tekulvw/mag-recomp-server/commit/b840db87731801903adeb5bb52490ce66e75d2fc))
* **medius:** accept 12-hour portal tickets and answer refusals ([#216](https://github.com/tekulvw/mag-recomp-server/issues/216)) ([aa91046](https://github.com/tekulvw/mag-recomp-server/commit/aa910468f686374be412b44627cefb6e7a627953))
* **medius:** place parties into running matches and cap parties at one squad ([#221](https://github.com/tekulvw/mag-recomp-server/issues/221)) ([322ca80](https://github.com/tekulvw/mag-recomp-server/commit/322ca8086bf8d5a4557c6cf60b6054dd09a00b32))


### Bug Fixes

* keep matches alive through short gameplay stalls ([#217](https://github.com/tekulvw/mag-recomp-server/issues/217)) ([344ec1e](https://github.com/tekulvw/mag-recomp-server/commit/344ec1ef8592c848ff550ac40313f8842cb89b4c))
* **medius:** accept never-awarded apparel in retail loadout saves ([#211](https://github.com/tekulvw/mag-recomp-server/issues/211)) ([1c2294e](https://github.com/tekulvw/mag-recomp-server/commit/1c2294e8997ef5ee7bd78f75736e9485796e9806))

## [0.7.1](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.7.0...medius-v0.7.1) (2026-11-24)


### Bug Fixes

* **medius:** answer lobby player cards and show online IDs in matches ([#209](https://github.com/tekulvw/mag-recomp-server/issues/209)) ([3e4b7eb](https://github.com/tekulvw/mag-recomp-server/commit/3e4b7eb535e86f0bc308d6609b710b77621027e8))
* **medius:** reconfigure the gameplay worker whenever it reconnects ([#208](https://github.com/tekulvw/mag-recomp-server/issues/208)) ([3ef3300](https://github.com/tekulvw/mag-recomp-server/commit/3ef3300b440521e75795d60c9eb09520941b6810))

## [0.7.0](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.6.0...medius-v0.7.0) (2026-11-23)


### Features

* **medius:** raise characters to an operator level on completing Training ([#205](https://github.com/tekulvw/mag-recomp-server/issues/205)) ([c6aa4bf](https://github.com/tekulvw/mag-recomp-server/commit/c6aa4bfbe93b7bbb64bbff7638bdd3a5a6b3d8d7))

## [0.6.0](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.5.0...medius-v0.6.0) (2026-11-23)


### Features

* **medius:** add an internal admin API for character maintenance ([#202](https://github.com/tekulvw/mag-recomp-server/issues/202)) ([ef28e6f](https://github.com/tekulvw/mag-recomp-server/commit/ef28e6ff096985d9057bf10b4b4d420448312579))

## [0.5.0](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.4.0...medius-v0.5.0) (2026-11-23)


### Features

* **gameplay:** add bounded Domination profiles and controller primitives ([#83](https://github.com/tekulvw/mag-recomp-server/issues/83)) ([bb3a715](https://github.com/tekulvw/mag-recomp-server/commit/bb3a7157907b8ca598f65b878968f612664f6f79))
* **gameplay:** award contextual FRAGO bonuses for healing and revival ([3903542](https://github.com/tekulvw/mag-recomp-server/commit/3903542961923dcf1560c38d91b4d338291be7a3))
* **gameplay:** award contextual FRAGO kill bonuses ([9ec1466](https://github.com/tekulvw/mag-recomp-server/commit/9ec1466486dc67e20a197d9614f09c7433f34c2f))
* **gameplay:** award FRAGO bonuses for healing and revival ([4a0b353](https://github.com/tekulvw/mag-recomp-server/commit/4a0b353bfb595229f3b4dd53d0cd62ec065519c9))
* **gameplay:** award repair work XP and contextual FRAGO bonuses ([be58260](https://github.com/tekulvw/mag-recomp-server/commit/be582602c905845db7bc57d75f1ff8c26f337f1d))
* **gameplay:** award repair work XP and FRAGO bonuses ([b64b625](https://github.com/tekulvw/mag-recomp-server/commit/b64b625b2b6e3774c018632895718398974eca8d))
* **gameplay:** award structural destruction and escort delivery XP ([c0d7448](https://github.com/tekulvw/mag-recomp-server/commit/c0d74480e3f6a756fd8597e303aee2bf35da8757))
* **gameplay:** award structural destruction and escort delivery XP ([decd9af](https://github.com/tekulvw/mag-recomp-server/commit/decd9afb3bf87a955a7898a9241f3ed936f850e6))
* **gameplay:** isolate completed match cohorts for requeue ([#105](https://github.com/tekulvw/mag-recomp-server/issues/105)) ([45a0d2f](https://github.com/tekulvw/mag-recomp-server/commit/45a0d2f52ea4839a4bf3c74a992f858f9d25b18f))
* **gameplay:** persist leadership points and rank advancement ([6b34889](https://github.com/tekulvw/mag-recomp-server/commit/6b3488993dd9118b39b8ac0b18ff175f160dfa1e))
* **gameplay:** settle objective XP with FRAGO bonuses ([999ef58](https://github.com/tekulvw/mag-recomp-server/commit/999ef58549ee8f3de1f816aa6f42b65c3f9e2581))
* **gameplay:** settle objective XP with FRAGO bonuses ([9419478](https://github.com/tekulvw/mag-recomp-server/commit/94194789188ec8d7e0add8e4c90e01bdf7df08dd))
* **horizon:** add opt-in retail party matchmaking ([#186](https://github.com/tekulvw/mag-recomp-server/issues/186)) ([2d1be8a](https://github.com/tekulvw/mag-recomp-server/commit/2d1be8a3db5d401e8cf3c0cdfd3e341b5c927fac))
* **medius:** verify portal game tickets ([#195](https://github.com/tekulvw/mag-recomp-server/issues/195)) ([2b7f4f6](https://github.com/tekulvw/mag-recomp-server/commit/2b7f4f62f146e0edf87750c9a1dbd4d1f0c12b0e))
* **progression:** award authored human assist experience ([#162](https://github.com/tekulvw/mag-recomp-server/issues/162)) ([d58af6d](https://github.com/tekulvw/mag-recomp-server/commit/d58af6d76cfccc6101eeba1bce51e64c58fb73a8))
* **server:** coordinate configured independent live worlds ([#106](https://github.com/tekulvw/mag-recomp-server/issues/106)) ([d5e68a4](https://github.com/tekulvw/mag-recomp-server/commit/d5e68a488892e7c81731bf05081bc1c0a9e2687f))


### Bug Fixes

* **armory:** support durable retail loadout deletion ([#192](https://github.com/tekulvw/mag-recomp-server/issues/192)) ([9a4625a](https://github.com/tekulvw/mag-recomp-server/commit/9a4625aca9e6448e80e29156c7bde6d56dad3cde))
* **medius:** keep party cancellation out of the solo queue ([#182](https://github.com/tekulvw/mag-recomp-server/issues/182)) ([541d1f0](https://github.com/tekulvw/mag-recomp-server/commit/541d1f0c339caaa49b41998769981e6bb95b7ffa))

## [0.4.0](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.3.1...medius-v0.4.0) (2026-11-11)


### Features

* **medius:** optional starting level for new characters ([#77](https://github.com/tekulvw/mag-recomp-server/issues/77)) ([c93929d](https://github.com/tekulvw/mag-recomp-server/commit/c93929d05e4cb5c0efd163ac275e5327f131ea94))

## [0.3.1](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.3.0...medius-v0.3.1) (2026-11-06)


### Bug Fixes

* **medius:** keep Program.Main as the server entry point ([#72](https://github.com/tekulvw/mag-recomp-server/issues/72)) ([075e9e7](https://github.com/tekulvw/mag-recomp-server/commit/075e9e7a2d543710acbb1e2b7e1cf0025d63916a))

## [0.3.0](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.2.4...medius-v0.3.0) (2026-11-06)


### Features

* **medius:** add mag-admin for account and character maintenance ([#70](https://github.com/tekulvw/mag-recomp-server/issues/70)) ([ddb2572](https://github.com/tekulvw/mag-recomp-server/commit/ddb2572e6745cda3d62d4754da0775b3aaf6f340))

## [0.2.4](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.2.3...medius-v0.2.4) (2026-11-06)


### Bug Fixes

* **medius:** name and log rejected join requests ([#65](https://github.com/tekulvw/mag-recomp-server/issues/65)) ([7adc12d](https://github.com/tekulvw/mag-recomp-server/commit/7adc12da1a96a0487c14d4a762dcd2962829bfbf))

## [0.2.3](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.2.2...medius-v0.2.3) (2026-11-06)


### Bug Fixes

* **medius:** name and log refused gameplay admissions ([#58](https://github.com/tekulvw/mag-recomp-server/issues/58)) ([4bb41e8](https://github.com/tekulvw/mag-recomp-server/commit/4bb41e8befee7324caffd427bc3ce0ee3345ece9))

## [0.2.2](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.2.1...medius-v0.2.2) (2026-11-06)


### Bug Fixes

* **medius:** stop rejected gameplay releases from blocking rejoin ([#57](https://github.com/tekulvw/mag-recomp-server/issues/57)) ([f306c13](https://github.com/tekulvw/mag-recomp-server/commit/f306c13bcad8b9683dcff819d5518582a94e169c))

## [0.2.1](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.2.0...medius-v0.2.1) (2026-11-04)


### Bug Fixes

* **medius:** accept NP identities bound to the authenticated account ([#47](https://github.com/tekulvw/mag-recomp-server/issues/47)) ([218c81e](https://github.com/tekulvw/mag-recomp-server/commit/218c81e77b335e2bffa59d7cd46b11dc8e6d96cd))

## [0.2.0](https://github.com/tekulvw/mag-recomp-server/compare/medius-v0.1.0...medius-v0.2.0) (2026-11-04)


### Features

* bundle deployed server changelogs into in-game NEWS ([#42](https://github.com/tekulvw/mag-recomp-server/issues/42)) ([afb360e](https://github.com/tekulvw/mag-recomp-server/commit/afb360edf93156ad9f3c6842794cfab534ce3ace))
* **gameplay:** add typed human incapacitation and revive ([a79375e](https://github.com/tekulvw/mag-recomp-server/commit/a79375ec761de8f188e988f9aa902c01291a5f6b))
* **gameplay:** admit bounded retail Acquisition matches ([#35](https://github.com/tekulvw/mag-recomp-server/issues/35)) ([e311237](https://github.com/tekulvw/mag-recomp-server/commit/e3112374f797689bcb2976c2f05db7242beb44ad))
* **gameplay:** persist committed in-match kit preferences ([a719eb3](https://github.com/tekulvw/mag-recomp-server/commit/a719eb3f48f09cc2bdab9f72c473ab32faa47ff5))
* **gameplay:** refresh active saved armory kits ([f617952](https://github.com/tekulvw/mag-recomp-server/commit/f6179524cde6dd4e6682e52c944f96766938c9a7))
* **gameplay:** require configurable ready-human team quorum ([e9e434c](https://github.com/tekulvw/mag-recomp-server/commit/e9e434c0a7f2635452c30292fcac298628eb8729))
* **matchmaking:** form faction-aware matches from human queues ([#37](https://github.com/tekulvw/mag-recomp-server/issues/37)) ([fda3b37](https://github.com/tekulvw/mag-recomp-server/commit/fda3b3738e78d3ae2a6f718fc3bf7eea1c7b71d7))


### Bug Fixes

* **medius:** preserve owned kit preferences safely ([5cf81b7](https://github.com/tekulvw/mag-recomp-server/commit/5cf81b78110852ba160a1b31c6b27bbdab56a5d4))
