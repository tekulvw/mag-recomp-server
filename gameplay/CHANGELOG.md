# Changelog

## [0.11.0](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.10.0...gameplay-v0.11.0) (2026-11-24)


### Features

* **gameplay:** let bots notice, hear and react to threats while busy ([#234](https://github.com/tekulvw/mag-recomp-server/issues/234)) ([39cb853](https://github.com/tekulvw/mag-recomp-server/commit/39cb853f3b8f21ad2975f037232c739224808af9))
* **gameplay:** send bots to defend and defuse armed objectives ([#240](https://github.com/tekulvw/mag-recomp-server/issues/240)) ([89b2513](https://github.com/tekulvw/mag-recomp-server/commit/89b25135636be472c0e556dd486f978e176446d6))


### Bug Fixes

* **gameplay:** let Sabotage frag grenades damage Characters ([#238](https://github.com/tekulvw/mag-recomp-server/issues/238)) ([242fad7](https://github.com/tekulvw/mag-recomp-server/commit/242fad7d9157f8b959b8a2b0b32e7c6369d7745d))

## [0.10.0](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.9.0...gameplay-v0.10.0) (2026-11-24)


### Features

* **gameplay:** plan long bot routes in stages when the node pool runs out ([#222](https://github.com/tekulvw/mag-recomp-server/issues/222)) ([3ec1527](https://github.com/tekulvw/mag-recomp-server/commit/3ec1527e5cba1c12dbb25b55683c1cab39fb8763))
* **gameplay:** qualify bot static resupply catalogs for every Sabotage map ([#237](https://github.com/tekulvw/mag-recomp-server/issues/237)) ([ea4e1d3](https://github.com/tekulvw/mag-recomp-server/commit/ea4e1d3d030a94c43a0c76df8772dda1eb571eb9))
* **gameplay:** start bot lives at the squad's active spawns ([#224](https://github.com/tekulvw/mag-recomp-server/issues/224)) ([f6d0719](https://github.com/tekulvw/mag-recomp-server/commit/f6d0719156abe1ade91cbab49aa3bc22baec8158))
* **tools:** project authored incapacitation rules and downed body multipliers ([#232](https://github.com/tekulvw/mag-recomp-server/issues/232)) ([766d12a](https://github.com/tekulvw/mag-recomp-server/commit/766d12a6e3396e3d2c710e06d7e5dd4c6b3413d2))


### Bug Fixes

* **gameplay:** keep players in the match when a bullet kills a bot after the result ([#227](https://github.com/tekulvw/mag-recomp-server/issues/227)) ([9022218](https://github.com/tekulvw/mag-recomp-server/commit/9022218313ce1f1f00452535d0436f3d40011e39))
* **gameplay:** let Sabotage rocket launchers fly and deal damage ([#230](https://github.com/tekulvw/mag-recomp-server/issues/230)) ([79976be](https://github.com/tekulvw/mag-recomp-server/commit/79976bed99e6d26876993cb4488919c8b7542306))
* **gameplay:** let Sabotage squad leaders set FRAGO targets ([#233](https://github.com/tekulvw/mag-recomp-server/issues/233)) ([44b0844](https://github.com/tekulvw/mag-recomp-server/commit/44b084433947accf57eb60243ca083b4dd577857))
* **gameplay:** replicate smoke thrown from either equipment gear in Sabotage ([#228](https://github.com/tekulvw/mag-recomp-server/issues/228)) ([9475e42](https://github.com/tekulvw/mag-recomp-server/commit/9475e423bc3d1aaacf1f86c3b576ac57fa46c383))

## [0.9.0](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.8.1...gameplay-v0.9.0) (2026-11-24)


### Features

* **gameplay:** extend human bullet damage to every retail firearm ([#218](https://github.com/tekulvw/mag-recomp-server/issues/218)) ([6d66198](https://github.com/tekulvw/mag-recomp-server/commit/6d661983334e13a09925374cdab76c2016f39eca))
* **gameplay:** fill match profiles with per-profile bots and start them with one human ([#213](https://github.com/tekulvw/mag-recomp-server/issues/213)) ([b840db8](https://github.com/tekulvw/mag-recomp-server/commit/b840db87731801903adeb5bb52490ce66e75d2fc))
* **gameplay:** load whole-map navmeshes with hash-bound coverage sidecars ([#212](https://github.com/tekulvw/mag-recomp-server/issues/212)) ([e1fe923](https://github.com/tekulvw/mag-recomp-server/commit/e1fe92301d6b0e388fb03da282b31abb92b25f6a))
* **medius:** place parties into running matches and cap parties at one squad ([#221](https://github.com/tekulvw/mag-recomp-server/issues/221)) ([322ca80](https://github.com/tekulvw/mag-recomp-server/commit/322ca8086bf8d5a4557c6cf60b6054dd09a00b32))


### Bug Fixes

* keep matches alive through short gameplay stalls ([#217](https://github.com/tekulvw/mag-recomp-server/issues/217)) ([344ec1e](https://github.com/tekulvw/mag-recomp-server/commit/344ec1ef8592c848ff550ac40313f8842cb89b4c))

## [0.8.1](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.8.0...gameplay-v0.8.1) (2026-11-24)


### Bug Fixes

* **medius:** reconfigure the gameplay worker whenever it reconnects ([#208](https://github.com/tekulvw/mag-recomp-server/issues/208)) ([3ef3300](https://github.com/tekulvw/mag-recomp-server/commit/3ef3300b440521e75795d60c9eb09520941b6810))

## [0.8.0](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.7.1...gameplay-v0.8.0) (2026-11-23)


### Features

* apply hospital rocket damage to qualified Helo targets ([#119](https://github.com/tekulvw/mag-recomp-server/issues/119)) ([89a0b7a](https://github.com/tekulvw/mag-recomp-server/commit/89a0b7af284d3815b48114eea725238de8f069ac))
* complete retail frag slots and rocket Ground damage ([#116](https://github.com/tekulvw/mag-recomp-server/issues/116)) ([fc95305](https://github.com/tekulvw/mag-recomp-server/commit/fc95305e44fccaaabe253001bd60f33b83413cdf))
* **data:** support portable mode bundles ([#166](https://github.com/tekulvw/mag-recomp-server/issues/166)) ([7fbaace](https://github.com/tekulvw/mag-recomp-server/commit/7fbaacee689966906170861ffb13f499d2eae380))
* **domination:** add qualified hospital blast damage ([#123](https://github.com/tekulvw/mag-recomp-server/issues/123)) ([d645eda](https://github.com/tekulvw/mag-recomp-server/commit/d645eda889e234c358aad5375c38fe577d2667dd))
* enable source-bound retail frag Character damage ([#117](https://github.com/tekulvw/mag-recomp-server/issues/117)) ([2bca484](https://github.com/tekulvw/mag-recomp-server/commit/2bca484f87d4b93a4e7dcc34b4bc10b1d87958d1))
* **gameplay:** add authenticated manual squad orders ([fa83a4e](https://github.com/tekulvw/mag-recomp-server/commit/fa83a4ea4617df62bc2829776b64054152a62557))
* **gameplay:** add bot static resupply service ([#127](https://github.com/tekulvw/mag-recomp-server/issues/127)) ([aae6fdf](https://github.com/tekulvw/mag-recomp-server/commit/aae6fdf26a26ae5e87f8ce54c9f14f752eddfa83))
* **gameplay:** add bounded bot resupply walking approaches ([#128](https://github.com/tekulvw/mag-recomp-server/issues/128)) ([6fe5c9f](https://github.com/tekulvw/mag-recomp-server/commit/6fe5c9fbdef8f839a0ee1dd3d76312fa8eaf7257))
* **gameplay:** add bounded Domination objective and round runtime ([#84](https://github.com/tekulvw/mag-recomp-server/issues/84)) ([fb70714](https://github.com/tekulvw/mag-recomp-server/commit/fb70714d0991db4bbd2922269baa33c2cb1f8d0e))
* **gameplay:** add bounded Domination profiles and controller primitives ([#83](https://github.com/tekulvw/mag-recomp-server/issues/83)) ([bb3a715](https://github.com/tekulvw/mag-recomp-server/commit/bb3a7157907b8ca598f65b878968f612664f6f79))
* **gameplay:** add source-bound Domination APC gunners ([#111](https://github.com/tekulvw/mag-recomp-server/issues/111)) ([b012e44](https://github.com/tekulvw/mag-recomp-server/commit/b012e447901f840a5f727ea4172c3ab0d1adb3c7))
* **gameplay:** add source-bound hospital rocket prerequisites ([#114](https://github.com/tekulvw/mag-recomp-server/issues/114)) ([3e100a4](https://github.com/tekulvw/mag-recomp-server/commit/3e100a4875b1c9fb444ebd113a185b41fd78c665))
* **gameplay:** add stock launcher data and capsule query support ([#150](https://github.com/tekulvw/mag-recomp-server/issues/150)) ([6948c3f](https://github.com/tekulvw/mag-recomp-server/commit/6948c3fde34e0c5fc8b9c011f6544d7970a9f68f))
* **gameplay:** admit Acquisition APC manual repair ([#176](https://github.com/tekulvw/mag-recomp-server/issues/176)) ([3271971](https://github.com/tekulvw/mag-recomp-server/commit/3271971d185a100deae4e3cc5d04f96d803a44c7))
* **gameplay:** admit source-qualified regional walking meshes ([#139](https://github.com/tekulvw/mag-recomp-server/issues/139)) ([b164829](https://github.com/tekulvw/mag-recomp-server/commit/b164829997034621a4c620a88247f46c0b403b7c))
* **gameplay:** apply living leadership AutoHeal ([861af10](https://github.com/tekulvw/mag-recomp-server/commit/861af102ddf79faf99ffc934ddf8e241bb96f407))
* **gameplay:** apply occupied Escort leadership AutoRepair ([ea6ea29](https://github.com/tekulvw/mag-recomp-server/commit/ea6ea299e454cbe9cf4c134a2f7d17e10a9ae6c4))
* **gameplay:** apply retail structure strike receivers ([8640dc3](https://github.com/tekulvw/mag-recomp-server/commit/8640dc3734453060ca5aa4f9018c751c564c5a39))
* **gameplay:** apply retail structure strike receivers ([fce3baa](https://github.com/tekulvw/mag-recomp-server/commit/fce3baa6f610b7bffdf1ac7699af261f6264f295))
* **gameplay:** award contextual FRAGO bonuses for healing and revival ([3903542](https://github.com/tekulvw/mag-recomp-server/commit/3903542961923dcf1560c38d91b4d338291be7a3))
* **gameplay:** award contextual FRAGO kill bonuses ([9ec1466](https://github.com/tekulvw/mag-recomp-server/commit/9ec1466486dc67e20a197d9614f09c7433f34c2f))
* **gameplay:** award FRAGO bonuses for healing and revival ([4a0b353](https://github.com/tekulvw/mag-recomp-server/commit/4a0b353bfb595229f3b4dd53d0cd62ec065519c9))
* **gameplay:** award ordinary firearm assist counters ([#126](https://github.com/tekulvw/mag-recomp-server/issues/126)) ([a735c7c](https://github.com/tekulvw/mag-recomp-server/commit/a735c7cb7c0e8de6879cfcbdf21f4e3b21314bd0))
* **gameplay:** award repair work XP and contextual FRAGO bonuses ([be58260](https://github.com/tekulvw/mag-recomp-server/commit/be582602c905845db7bc57d75f1ff8c26f337f1d))
* **gameplay:** award repair work XP and FRAGO bonuses ([b64b625](https://github.com/tekulvw/mag-recomp-server/commit/b64b625b2b6e3774c018632895718398974eca8d))
* **gameplay:** award structural destruction and escort delivery XP ([c0d7448](https://github.com/tekulvw/mag-recomp-server/commit/c0d74480e3f6a756fd8597e303aee2bf35da8757))
* **gameplay:** award structural destruction and escort delivery XP ([decd9af](https://github.com/tekulvw/mag-recomp-server/commit/decd9afb3bf87a955a7898a9241f3ed936f850e6))
* **gameplay:** bind infantry launcher data and Acquisition contacts ([#151](https://github.com/tekulvw/mag-recomp-server/issues/151)) ([9199648](https://github.com/tekulvw/mag-recomp-server/commit/91996486bc2d1811ef01cf6206f23befd3ef271e))
* **gameplay:** complete Acquisition Structural interactions ([442b97d](https://github.com/tekulvw/mag-recomp-server/commit/442b97d400238502ee96a0c586712d59e901402a))
* **gameplay:** complete source-assigned Acquisition bot captures ([#140](https://github.com/tekulvw/mag-recomp-server/issues/140)) ([3258dd5](https://github.com/tekulvw/mag-recomp-server/commit/3258dd57466f0f1765112484e316dd2b93df4db2))
* **gameplay:** complete the core Acquisition match loop ([#81](https://github.com/tekulvw/mag-recomp-server/issues/81)) ([9b7cd13](https://github.com/tekulvw/mag-recomp-server/commit/9b7cd1389ea37f8065b13380f64508fd3bb121af))
* **gameplay:** connect Domination objectives and vehicle lifecycle ([#85](https://github.com/tekulvw/mag-recomp-server/issues/85)) ([11d0ef1](https://github.com/tekulvw/mag-recomp-server/commit/11d0ef157d260fb3f4f8ee66433d213b29d3b390))
* **gameplay:** connect platoon authority and command effects ([bb3586e](https://github.com/tekulvw/mag-recomp-server/commit/bb3586ed9f119189ca94094345014f845a5dc6f4))
* **gameplay:** enable Acquisition Escort gunners ([#113](https://github.com/tekulvw/mag-recomp-server/issues/113)) ([542db6b](https://github.com/tekulvw/mag-recomp-server/commit/542db6bcf7f53e102a87bfe81cfed64d732f2733))
* **gameplay:** enable hospital rockets against on-foot characters ([#115](https://github.com/tekulvw/mag-recomp-server/issues/115)) ([983dd27](https://github.com/tekulvw/mag-recomp-server/commit/983dd27dc6304291243695b50f72c29923a00542))
* **gameplay:** enable qualified Domination APC drivers ([#109](https://github.com/tekulvw/mag-recomp-server/issues/109)) ([3a4de1f](https://github.com/tekulvw/mag-recomp-server/commit/3a4de1f0c15f9844b35f5a6b3caf26c275249846))
* **gameplay:** enable squad abilities by default ([534ed89](https://github.com/tekulvw/mag-recomp-server/commit/534ed896c20c0d2a91ebe91b36ec71fabea3c9af))
* **gameplay:** enable squad abilities by default ([25c0126](https://github.com/tekulvw/mag-recomp-server/commit/25c0126e38145354e39da1de490c2eba837ab9e0))
* **gameplay:** execute source-bound squad abilities ([e395567](https://github.com/tekulvw/mag-recomp-server/commit/e395567b76b2e7bacdb72d54324f0a8f4d3ac760))
* **gameplay:** execute source-bound squad abilities ([c24b648](https://github.com/tekulvw/mag-recomp-server/commit/c24b64884cae1026430f1db21755647d249966a6))
* **gameplay:** guard regional bot movement against current bodies ([#134](https://github.com/tekulvw/mag-recomp-server/issues/134)) ([7367588](https://github.com/tekulvw/mag-recomp-server/commit/73675889a3697749f25861a35f6961e6618fd80b))
* **gameplay:** heal humans at static supply points ([#148](https://github.com/tekulvw/mag-recomp-server/issues/148)) ([f39f5f9](https://github.com/tekulvw/mag-recomp-server/commit/f39f5f94daf9082d62972b2ae3d0a57b4f387aac))
* **gameplay:** isolate completed match cohorts for requeue ([#105](https://github.com/tekulvw/mag-recomp-server/issues/105)) ([45a0d2f](https://github.com/tekulvw/mag-recomp-server/commit/45a0d2f52ea4839a4bf3c74a992f858f9d25b18f))
* **gameplay:** let Acquisition defender bots defuse charges ([#133](https://github.com/tekulvw/mag-recomp-server/issues/133)) ([cb99b42](https://github.com/tekulvw/mag-recomp-server/commit/cb99b42ec386943ab0ba0bee1bce4713e9e01a22))
* **gameplay:** persist leadership points and rank advancement ([6b34889](https://github.com/tekulvw/mag-recomp-server/commit/6b3488993dd9118b39b8ac0b18ff175f160dfa1e))
* **gameplay:** preserve detached Acquisition bot planting attribution ([#136](https://github.com/tekulvw/mag-recomp-server/issues/136)) ([785c73b](https://github.com/tekulvw/mag-recomp-server/commit/785c73b30b67affa811211f56e66cb7d1cf6704d))
* **gameplay:** qualify excluded static roots for Acquisition respawns ([#79](https://github.com/tekulvw/mag-recomp-server/issues/79)) ([508c7ea](https://github.com/tekulvw/mag-recomp-server/commit/508c7eaedadce2a4080c9c92ff24b77ca72f0c4f))
* **gameplay:** replenish human grenades at static supply points ([#147](https://github.com/tekulvw/mag-recomp-server/issues/147)) ([9fa7113](https://github.com/tekulvw/mag-recomp-server/commit/9fa7113e3cd83daed3fae85cf128c6dc871ea8cc))
* **gameplay:** reserve party cohorts atomically ([#185](https://github.com/tekulvw/mag-recomp-server/issues/185)) ([60e8b0b](https://github.com/tekulvw/mag-recomp-server/commit/60e8b0bdf7f0ad9857df60a41ec525d7d4f4934d))
* **gameplay:** reuse certified drops for bot resupply ([#129](https://github.com/tekulvw/mag-recomp-server/issues/129)) ([253f7c9](https://github.com/tekulvw/mag-recomp-server/commit/253f7c961460d0bd123e57bded1038f957eaa324))
* **gameplay:** settle objective XP with FRAGO bonuses ([999ef58](https://github.com/tekulvw/mag-recomp-server/commit/999ef58549ee8f3de1f816aa6f42b65c3f9e2581))
* **gameplay:** settle objective XP with FRAGO bonuses ([9419478](https://github.com/tekulvw/mag-recomp-server/commit/94194789188ec8d7e0add8e4c90e01bdf7df08dd))
* **gameplay:** support Acquisition APC crew ([#174](https://github.com/tekulvw/mag-recomp-server/issues/174)) ([95c1de5](https://github.com/tekulvw/mag-recomp-server/commit/95c1de5832119deac2d8bb1d62d7564ef0d0dba4))
* **gameplay:** support explicit single-team regional bot allocation ([#138](https://github.com/tekulvw/mag-recomp-server/issues/138)) ([1eed2b7](https://github.com/tekulvw/mag-recomp-server/commit/1eed2b7f1339e5945dbe89bf29071dd98d49226e))
* **gameplay:** support guided infantry launchers ([#173](https://github.com/tekulvw/mag-recomp-server/issues/173)) ([26d935a](https://github.com/tekulvw/mag-recomp-server/commit/26d935a856d4a042b64e165bd6e069a8b77d3b8f))
* **gameplay:** support indexed Domination hospital primary turrets ([#110](https://github.com/tekulvw/mag-recomp-server/issues/110)) ([bcd4ac6](https://github.com/tekulvw/mag-recomp-server/commit/bcd4ac6c551ac8442bbd8cdf4de0b550dd8e0091))
* **gameplay:** support manual Ground vehicle repairs ([36d45ff](https://github.com/tekulvw/mag-recomp-server/commit/36d45ffee4334dc716745bc7ba0f9ba2a4488d80))
* **gameplay:** support manual Ground vehicle repairs ([907250a](https://github.com/tekulvw/mag-recomp-server/commit/907250ac7de63cc9fea1e1fa4b3d088e4bd472c1))
* **gameplay:** support native platoon command abilities ([815b971](https://github.com/tekulvw/mag-recomp-server/commit/815b9718a5d8f4d81215f41ab86e0002e13ad636))
* **gameplay:** support OIC strategics and Acquisition structural effects ([d9638b6](https://github.com/tekulvw/mag-recomp-server/commit/d9638b6461736edf8cb4ead6f4237ff01ba3f310))
* **gameplay:** support purchased poison grenades ([#168](https://github.com/tekulvw/mag-recomp-server/issues/168)) ([238d69a](https://github.com/tekulvw/mag-recomp-server/commit/238d69a359d8320e8ce222079a75630e35ef921c))
* **gameplay:** support stock and unlocked rifle launchers ([#164](https://github.com/tekulvw/mag-recomp-server/issues/164)) ([bd84e31](https://github.com/tekulvw/mag-recomp-server/commit/bd84e316f7bdb3d51eb22e3fd4ca1a9a51fd2c2b))
* **gameplay:** support stock Flash grenades ([#171](https://github.com/tekulvw/mag-recomp-server/issues/171)) ([158ab2c](https://github.com/tekulvw/mag-recomp-server/commit/158ab2c75a18a809457ef98380dcd48ca58fe298))
* **gameplay:** support stock infantry rockets in Acquisition and Domination ([#152](https://github.com/tekulvw/mag-recomp-server/issues/152)) ([da9197a](https://github.com/tekulvw/mag-recomp-server/commit/da9197afcb5785b68b5a3d8d8f986929c85534fa))
* **gameplay:** support stock rifle underbarrel launchers ([#161](https://github.com/tekulvw/mag-recomp-server/issues/161)) ([b97d091](https://github.com/tekulvw/mag-recomp-server/commit/b97d0914106d680af9820ab0a381e156546f1c75))
* **gameplay:** support stock smoke grenades in retail matches ([#153](https://github.com/tekulvw/mag-recomp-server/issues/153)) ([a46d363](https://github.com/tekulvw/mag-recomp-server/commit/a46d3639b3fb869ac3d134c47c145656393b104b))
* **navmesh:** bake full-world offline reference maps ([#131](https://github.com/tekulvw/mag-recomp-server/issues/131)) ([a6a18b7](https://github.com/tekulvw/mag-recomp-server/commit/a6a18b7ce4ece1c11cde95c6cb7727c8e3abdaa4))
* **navmesh:** support explicit source-bound movement slope profiles ([#135](https://github.com/tekulvw/mag-recomp-server/issues/135)) ([4e7162f](https://github.com/tekulvw/mag-recomp-server/commit/4e7162fbf493c1f4a74fb63bb80dfedef3cac954))
* **progression:** award authored human assist experience ([#162](https://github.com/tekulvw/mag-recomp-server/issues/162)) ([d58af6d](https://github.com/tekulvw/mag-recomp-server/commit/d58af6d76cfccc6101eeba1bce51e64c58fb73a8))
* **server:** coordinate configured independent live worlds ([#106](https://github.com/tekulvw/mag-recomp-server/issues/106)) ([d5e68a4](https://github.com/tekulvw/mag-recomp-server/commit/d5e68a488892e7c81731bf05081bc1c0a9e2687f))
* support Acquisition frag damage with source material cover ([#118](https://github.com/tekulvw/mag-recomp-server/issues/118)) ([f6e1d97](https://github.com/tekulvw/mag-recomp-server/commit/f6e1d97b9e72bdae7d75c50aaec3024c93620995))


### Bug Fixes

* **acquisition:** qualify ordinary frag damage to ground vehicles ([#120](https://github.com/tekulvw/mag-recomp-server/issues/120)) ([18f2782](https://github.com/tekulvw/mag-recomp-server/commit/18f2782d9a61e08cd679a1ad898c2223fd50d70e))
* admit source-qualified retail firearm modes ([#163](https://github.com/tekulvw/mag-recomp-server/issues/163)) ([8e09105](https://github.com/tekulvw/mag-recomp-server/commit/8e091054054ba4f9b8e238639ff1bd61c0ad431d))
* **armory:** support durable retail loadout deletion ([#192](https://github.com/tekulvw/mag-recomp-server/issues/192)) ([9a4625a](https://github.com/tekulvw/mag-recomp-server/commit/9a4625aca9e6448e80e29156c7bde6d56dad3cde))
* **collision:** certify world sweep contact boundaries ([#175](https://github.com/tekulvw/mag-recomp-server/issues/175)) ([90400f9](https://github.com/tekulvw/mag-recomp-server/commit/90400f97b265f0943fd1966c08f5e22b3515fc7f))
* **domination:** qualify ordinary frag ground vehicle damage ([#121](https://github.com/tekulvw/mag-recomp-server/issues/121)) ([52d064f](https://github.com/tekulvw/mag-recomp-server/commit/52d064f868ab468913735d6ca40fbaacd0c05ee8))
* **domination:** qualify ordinary frag helo damage ([#122](https://github.com/tekulvw/mag-recomp-server/issues/122)) ([15bb3dd](https://github.com/tekulvw/mag-recomp-server/commit/15bb3dd1f59e3e5b99ad3f0e1352b8627421dc57))
* **gameplay:** account for bot firearm kills and deaths ([#125](https://github.com/tekulvw/mag-recomp-server/issues/125)) ([c5303ac](https://github.com/tekulvw/mag-recomp-server/commit/c5303ac97628260e9bd94418eef0c0aae71e50ed))
* **gameplay:** admit APC and on-foot Helo hits ([#178](https://github.com/tekulvw/mag-recomp-server/issues/178)) ([581e467](https://github.com/tekulvw/mag-recomp-server/commit/581e467cfad39c5f0e24ec36c20147efbf87eb40))
* **gameplay:** admit on-foot hits against Acquisition APCs ([#177](https://github.com/tekulvw/mag-recomp-server/issues/177)) ([4601771](https://github.com/tekulvw/mag-recomp-server/commit/4601771cea21aec04f7ea00e33db6b06127ec6c3))
* **gameplay:** admit qualified ordinary weapon attachments ([#165](https://github.com/tekulvw/mag-recomp-server/issues/165)) ([7d29131](https://github.com/tekulvw/mag-recomp-server/commit/7d291311f60bd8af9256f4e07b07d29faa91bddf))
* **gameplay:** allow Hospital primary hits on Helos ([#172](https://github.com/tekulvw/mag-recomp-server/issues/172)) ([8a24ba3](https://github.com/tekulvw/mag-recomp-server/commit/8a24ba3f9842430e094d91f04437dd3c5f3d4965))
* **gameplay:** allow owned redeploy from Ground seats ([#143](https://github.com/tekulvw/mag-recomp-server/issues/143)) ([7bd07ee](https://github.com/tekulvw/mag-recomp-server/commit/7bd07ee8332ae88ade88675896986d2448cf43ca))
* **gameplay:** award current Ground drivers kill assists ([#189](https://github.com/tekulvw/mag-recomp-server/issues/189)) ([7a8b9dd](https://github.com/tekulvw/mag-recomp-server/commit/7a8b9dd060dd5996cb1010d3877f5d48a315406d))
* **gameplay:** batch initial objective control snapshots ([#170](https://github.com/tekulvw/mag-recomp-server/issues/170)) ([27abc0a](https://github.com/tekulvw/mag-recomp-server/commit/27abc0a78076f978cabb405be4a986f5e667957b))
* **gameplay:** complete objective counters and rebuilt defuse rewards ([1af7194](https://github.com/tekulvw/mag-recomp-server/commit/1af71945c8c25332a31538dfb73ebc5aa6cc39af))
* **gameplay:** complete objective counters and rebuilt defuse rewards ([9cdee8e](https://github.com/tekulvw/mag-recomp-server/commit/9cdee8ed7fc9d3fdab352c19735a20885818f87f))
* **gameplay:** count ordinary frag assists against human victims ([#149](https://github.com/tekulvw/mag-recomp-server/issues/149)) ([ecbfb97](https://github.com/tekulvw/mag-recomp-server/commit/ecbfb976252adc6497f05fec9f1c5357314dee82))
* **gameplay:** count qualified frag deaths for registry bots ([#130](https://github.com/tekulvw/mag-recomp-server/issues/130)) ([8abb2ce](https://github.com/tekulvw/mag-recomp-server/commit/8abb2ceecc5ed3cd5e20877811099e1475639840))
* **gameplay:** credit mixed rocket combat assists ([#154](https://github.com/tekulvw/mag-recomp-server/issues/154)) ([fed028d](https://github.com/tekulvw/mag-recomp-server/commit/fed028d48dc6d47d3601ee1c2d4282a9ae85e591))
* **gameplay:** enable operational allied Helo repair ([#169](https://github.com/tekulvw/mag-recomp-server/issues/169)) ([3120e63](https://github.com/tekulvw/mag-recomp-server/commit/3120e63eb00544695c50a2058d3b8f12d100a919))
* **gameplay:** finish Helo and Hospital redeploy cleanup ([#144](https://github.com/tekulvw/mag-recomp-server/issues/144)) ([d6bc93c](https://github.com/tekulvw/mag-recomp-server/commit/d6bc93c8ff004485fd63b2b751fbc3f0d1bc8383))
* **gameplay:** heal qualified mounted characters ([#190](https://github.com/tekulvw/mag-recomp-server/issues/190)) ([43dfd6b](https://github.com/tekulvw/mag-recomp-server/commit/43dfd6b57b87c9d30463cda36d86b2f6f9e876ea))
* **gameplay:** honor held leadership for squad spawn selection ([5531077](https://github.com/tekulvw/mag-recomp-server/commit/5531077e34fcc725ec49fa770bbd6ecdf7287791))
* **gameplay:** let human Medkits heal living allied bots ([#180](https://github.com/tekulvw/mag-recomp-server/issues/180)) ([05c383a](https://github.com/tekulvw/mag-recomp-server/commit/05c383a57160e0e5899b6f2465193b792d0ec26b))
* **gameplay:** preserve retail vehicle repair pulses ([#187](https://github.com/tekulvw/mag-recomp-server/issues/187)) ([e2e7249](https://github.com/tekulvw/mag-recomp-server/commit/e2e724927297161c642a8e46386b1ce09d0405b2))
* **gameplay:** publish human falling death notifications ([#142](https://github.com/tekulvw/mag-recomp-server/issues/142)) ([474a698](https://github.com/tekulvw/mag-recomp-server/commit/474a698437806f5db9482356164b46e2effde90d))
* **gameplay:** publish human knife death notifications ([#141](https://github.com/tekulvw/mag-recomp-server/issues/141)) ([9054d7c](https://github.com/tekulvw/mag-recomp-server/commit/9054d7cfa958ab9928216bc8a58e2afa60d6a591))
* **gameplay:** publish inherited leadership tiers ([#179](https://github.com/tekulvw/mag-recomp-server/issues/179)) ([953ec70](https://github.com/tekulvw/mag-recomp-server/commit/953ec70dc724902826f73b816433410ff30d30a8))
* **gameplay:** publish launch-bound rocket death notices ([#184](https://github.com/tekulvw/mag-recomp-server/issues/184)) ([e661e72](https://github.com/tekulvw/mag-recomp-server/commit/e661e7258325e89017cb2b705041a67a5eb00a67))
* **gameplay:** publish mounted primary death notices ([#183](https://github.com/tekulvw/mag-recomp-server/issues/183)) ([b9210b9](https://github.com/tekulvw/mag-recomp-server/commit/b9210b903ce8e4a8dcc96ef1874fbb046048d1b1))
* **gameplay:** publish same-state Domination playback sequences ([#108](https://github.com/tekulvw/mag-recomp-server/issues/108)) ([387c4e7](https://github.com/tekulvw/mag-recomp-server/commit/387c4e7ea6137be18e0b4ac09b695d97c6a7f593))
* **gameplay:** qualify Domination APC passenger area scans ([#107](https://github.com/tekulvw/mag-recomp-server/issues/107)) ([5dc3c4f](https://github.com/tekulvw/mag-recomp-server/commit/5dc3c4fa666f8e453a7716f694a28507d8dd1ab0))
* **gameplay:** qualify scripted objective reward attribution ([0c75840](https://github.com/tekulvw/mag-recomp-server/commit/0c75840a0272e452d8012ca5d40d8c4c5cf12955))
* **gameplay:** qualify scripted objective reward attribution ([db09b0b](https://github.com/tekulvw/mag-recomp-server/commit/db09b0bc9c90a167c2dc31cddc8526626bb16ef8))
* **gameplay:** restore occupied APC AutoRepair ([#181](https://github.com/tekulvw/mag-recomp-server/issues/181)) ([f2477d4](https://github.com/tekulvw/mag-recomp-server/commit/f2477d4d426566c0d85a3a99b9c4349f63f8c717))
* **gameplay:** retain assault rifle damage with compatible grips ([#157](https://github.com/tekulvw/mag-recomp-server/issues/157)) ([d5f2f83](https://github.com/tekulvw/mag-recomp-server/commit/d5f2f83b179bf0d356f45a0676d47a4b43fa405f))
* **gameplay:** retain assault rifle damage with compatible suppressors ([#156](https://github.com/tekulvw/mag-recomp-server/issues/156)) ([7788af8](https://github.com/tekulvw/mag-recomp-server/commit/7788af8796c2a627d2a4fe697925401c277b879c))
* **gameplay:** retain reconnect stats without progression ([#188](https://github.com/tekulvw/mag-recomp-server/issues/188)) ([fcebaeb](https://github.com/tekulvw/mag-recomp-server/commit/fcebaeb077d1b4071efd847523df2ea4a22f0db5))
* **gameplay:** retain retail LMG damage with compatible bipods ([#155](https://github.com/tekulvw/mag-recomp-server/issues/155)) ([3331e81](https://github.com/tekulvw/mag-recomp-server/commit/3331e81a31ce558bf722ba96246bb7a615267c13))
* **gameplay:** retain rifle damage with underbarrel launchers ([#159](https://github.com/tekulvw/mag-recomp-server/issues/159)) ([0f8b68b](https://github.com/tekulvw/mag-recomp-server/commit/0f8b68b2f3b5cce46ca3967e7376a88e403d1c92))
* **gameplay:** retain sidearm damage with compatible suppressors ([#158](https://github.com/tekulvw/mag-recomp-server/issues/158)) ([1d38f88](https://github.com/tekulvw/mag-recomp-server/commit/1d38f88c8c8eea8318fb3cf53e8202b369bbcea0))
* **gameplay:** route mounted primaries through Ground damage ([#112](https://github.com/tekulvw/mag-recomp-server/issues/112)) ([94773c1](https://github.com/tekulvw/mag-recomp-server/commit/94773c1cafdf43b65d1cf7063f44c1ad63f64ed2))
* **gameplay:** separate secure and demolition action timing ([#124](https://github.com/tekulvw/mag-recomp-server/issues/124)) ([95bc851](https://github.com/tekulvw/mag-recomp-server/commit/95bc8519df95c0292b9ab64f7664a24e5375a892))
* **gameplay:** settle airborne bots inside their walking corridor ([#137](https://github.com/tekulvw/mag-recomp-server/issues/137)) ([6b32e36](https://github.com/tekulvw/mag-recomp-server/commit/6b32e362522dfe2ff29313edf1b837010a096f0c))
* **gameplay:** support Sabotage defender objective control ([#191](https://github.com/tekulvw/mag-recomp-server/issues/191)) ([ec6f64e](https://github.com/tekulvw/mag-recomp-server/commit/ec6f64e7acd25de5064816473a9190134545dd81))
* **navmesh:** complete bounded reference route queries ([#132](https://github.com/tekulvw/mag-recomp-server/issues/132)) ([b59206b](https://github.com/tekulvw/mag-recomp-server/commit/b59206b8bf2725442f4432fd294ceed1afe72806))

## [0.7.1](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.7.0...gameplay-v0.7.1) (2026-11-08)


### Bug Fixes

* **gameplay:** check seeded exit candidates against every drop on the spawn island ([#75](https://github.com/tekulvw/mag-recomp-server/issues/75)) ([1f21d24](https://github.com/tekulvw/mag-recomp-server/commit/1f21d24cccb0053f79b1894e837953009b30514c))
* **gameplay:** stop queued bots deadlocking while walking to holding spots ([#74](https://github.com/tekulvw/mag-recomp-server/issues/74)) ([bb4e558](https://github.com/tekulvw/mag-recomp-server/commit/bb4e55866d3bf6b6882de6733539e93a3bbfac20))

## [0.7.0](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.6.0...gameplay-v0.7.0) (2026-11-06)


### Features

* **gameplay:** headless bot match simulation and spawn exit diagnostics ([#68](https://github.com/tekulvw/mag-recomp-server/issues/68)) ([c54f142](https://github.com/tekulvw/mag-recomp-server/commit/c54f14266fedce0e9b01c6bd7d3f05603e7aa4ce))
* **gameplay:** route spawned bots to the fastest exit before their goal ([#67](https://github.com/tekulvw/mag-recomp-server/issues/67)) ([5c8487b](https://github.com/tekulvw/mag-recomp-server/commit/5c8487b27d79082b073635239f45c054407276bd))

## [0.6.0](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.5.2...gameplay-v0.6.0) (2026-11-06)


### Features

* **gameplay:** add a bounded first spawn-wave delay override ([#64](https://github.com/tekulvw/mag-recomp-server/issues/64)) ([1e2d95e](https://github.com/tekulvw/mag-recomp-server/commit/1e2d95ecbb0800137d6093b5d173517316614775))

## [0.5.2](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.5.1...gameplay-v0.5.2) (2026-11-06)


### Bug Fixes

* **gameplay:** stop reserve replies carrying peers' queued packets ([#62](https://github.com/tekulvw/mag-recomp-server/issues/62)) ([5819cda](https://github.com/tekulvw/mag-recomp-server/commit/5819cda4a98edf7ebb4fea00076a3b894a5504b6))

## [0.5.1](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.5.0...gameplay-v0.5.1) (2026-11-06)


### Bug Fixes

* **gameplay:** keep loading humans from being evicted by bot traffic ([#61](https://github.com/tekulvw/mag-recomp-server/issues/61)) ([ebc1b21](https://github.com/tekulvw/mag-recomp-server/commit/ebc1b21c6c6fb47df3d71d03a43c57e57756862d))
* **gameplay:** stop landed convoy members re-queueing and restore spawn-floor staging ([#59](https://github.com/tekulvw/mag-recomp-server/issues/59)) ([2fa9096](https://github.com/tekulvw/mag-recomp-server/commit/2fa90960a11fa83866697dcd0d0818de28a023d0))

## [0.5.0](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.4.1...gameplay-v0.5.0) (2026-11-06)


### Features

* **gameplay:** spread spawn waves over every exit and share drop passages ([#56](https://github.com/tekulvw/mag-recomp-server/issues/56)) ([1bc1be3](https://github.com/tekulvw/mag-recomp-server/commit/1bc1be3de86001a0e3d5803ee395e41ccfe4ed93))


### Bug Fixes

* **medius:** stop rejected gameplay releases from blocking rejoin ([#57](https://github.com/tekulvw/mag-recomp-server/issues/57)) ([f306c13](https://github.com/tekulvw/mag-recomp-server/commit/f306c13bcad8b9683dcff819d5518582a94e169c))

## [0.4.1](https://github.com/tekulvw/mag-recomp-server/compare/gameplay-v0.4.0...gameplay-v0.4.1) (2026-11-04)


### Bug Fixes

* **gameplay:** range objective interactions by authored trigger boxes ([e80be93](https://github.com/tekulvw/mag-recomp-server/commit/e80be93f05ab1314a8f7aa6cfae7301d71bf5a1b))
* **gameplay:** rotate evidence logs with time-based retention ([#52](https://github.com/tekulvw/mag-recomp-server/issues/52)) ([d202178](https://github.com/tekulvw/mag-recomp-server/commit/d202178d14ddde656aa0d8aba7670bc1ddceb7a9))

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
