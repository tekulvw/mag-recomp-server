# Changelog

## [0.14.0](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.13.1...chart-v0.14.0) (2026-11-28)


### Features

* **medius:** cap respec fees so a matching floor allows unlimited respecs ([#432](https://github.com/tekulvw/mag-recomp-server/issues/432)) ([6b67b81](https://github.com/tekulvw/mag-recomp-server/commit/6b67b818b79f209aaaf29d1347717583840ef006))
* **medius:** handle skill respecs and add an operator respec point floor ([#428](https://github.com/tekulvw/mag-recomp-server/issues/428)) ([c2917d9](https://github.com/tekulvw/mag-recomp-server/commit/c2917d9c6e5e67df392c6dc054e932de8df3ca88))

## [0.13.1](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.13.0...chart-v0.13.1) (2026-11-26)


### Bug Fixes

* **chart:** give gameplay Pods a drain and a matching grace period ([#359](https://github.com/tekulvw/mag-recomp-server/issues/359)) ([b5f4716](https://github.com/tekulvw/mag-recomp-server/commit/b5f4716e33d99190189d79a041dd1b1c1ac53722))

## [0.13.0](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.12.1...chart-v0.13.0) (2026-11-26)


### Features

* **medius:** keep characters in PostgreSQL behind MAG_CHARACTER_STORE ([#315](https://github.com/tekulvw/mag-recomp-server/issues/315)) ([2e79200](https://github.com/tekulvw/mag-recomp-server/commit/2e792009dd3571fe64d137f7421d985321f33d20))


### Performance Improvements

* give each recipient its own bridge reply allowance behind a worker flag ([#318](https://github.com/tekulvw/mag-recomp-server/issues/318)) ([d12d67f](https://github.com/tekulvw/mag-recomp-server/commit/d12d67f2da4afb8e52c8a73ca5f409341828c8b9))

## [0.12.1](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.12.0...chart-v0.12.1) (2026-11-26)


### Bug Fixes

* **medius:** expire abandoned open reward rows so they cannot fill the character store ([#305](https://github.com/tekulvw/mag-recomp-server/issues/305)) ([3219c3e](https://github.com/tekulvw/mag-recomp-server/commit/3219c3ec122f64b852e095c05eba12691d6cfd57))

## [0.12.0](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.11.0...chart-v0.12.0) (2026-11-25)


### Features

* **medius:** admit humans against a human cap separate from world size ([#263](https://github.com/tekulvw/mag-recomp-server/issues/263)) ([0e853aa](https://github.com/tekulvw/mag-recomp-server/commit/0e853aa115f5975c9c6c6b9a18737cfbe9afe7e9))

## [0.11.0](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.10.0...chart-v0.11.0) (2026-11-24)


### Features

* **medius:** adopt a restarted registry worker without restarting Medius ([#255](https://github.com/tekulvw/mag-recomp-server/issues/255)) ([983b5b9](https://github.com/tekulvw/mag-recomp-server/commit/983b5b9681afe336ca78dbf0df5caa4903b8bfa7))
* **medius:** stop admitting players beyond the forwarded DME ports ([#253](https://github.com/tekulvw/mag-recomp-server/issues/253)) ([ff401f1](https://github.com/tekulvw/mag-recomp-server/commit/ff401f1353e3b8454b933d9a503a6d522f77e92d))

## [0.10.0](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.9.0...chart-v0.10.0) (2026-11-24)


### Features

* **chart:** run configured gameplay workers behind the world registry ([#214](https://github.com/tekulvw/mag-recomp-server/issues/214)) ([6fb825f](https://github.com/tekulvw/mag-recomp-server/commit/6fb825fc621f84a12ab41c88e3541fee5ce783d1))

## [0.9.0](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.8.0...chart-v0.9.0) (2026-11-23)


### Features

* **medius:** raise characters to an operator level on completing Training ([#205](https://github.com/tekulvw/mag-recomp-server/issues/205)) ([c6aa4bf](https://github.com/tekulvw/mag-recomp-server/commit/c6aa4bfbe93b7bbb64bbff7638bdd3a5a6b3d8d7))

## [0.8.0](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.7.0...chart-v0.8.0) (2026-11-23)


### Features

* **medius:** add an internal admin API for character maintenance ([#202](https://github.com/tekulvw/mag-recomp-server/issues/202)) ([ef28e6f](https://github.com/tekulvw/mag-recomp-server/commit/ef28e6ff096985d9057bf10b4b4d420448312579))

## [0.7.0](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.6.0...chart-v0.7.0) (2026-11-23)


### Features

* **data:** support portable mode bundles ([#166](https://github.com/tekulvw/mag-recomp-server/issues/166)) ([7fbaace](https://github.com/tekulvw/mag-recomp-server/commit/7fbaacee689966906170861ffb13f499d2eae380))
* **medius:** verify portal game tickets ([#195](https://github.com/tekulvw/mag-recomp-server/issues/195)) ([2b7f4f6](https://github.com/tekulvw/mag-recomp-server/commit/2b7f4f62f146e0edf87750c9a1dbd4d1f0c12b0e))


### Bug Fixes

* **chart:** accept Acquisition game mode ([#167](https://github.com/tekulvw/mag-recomp-server/issues/167)) ([4304886](https://github.com/tekulvw/mag-recomp-server/commit/430488644352cd59f40d1fbb4191368db09c09aa))

## [0.6.0](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.5.0...chart-v0.6.0) (2026-11-11)


### Features

* **medius:** optional starting level for new characters ([#77](https://github.com/tekulvw/mag-recomp-server/issues/77)) ([c93929d](https://github.com/tekulvw/mag-recomp-server/commit/c93929d05e4cb5c0efd163ac275e5327f131ea94))

## [0.5.0](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.4.2...chart-v0.5.0) (2026-11-06)


### Features

* **gameplay:** add a bounded first spawn-wave delay override ([#64](https://github.com/tekulvw/mag-recomp-server/issues/64)) ([1e2d95e](https://github.com/tekulvw/mag-recomp-server/commit/1e2d95ecbb0800137d6093b5d173517316614775))

## [0.4.2](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.4.1...chart-v0.4.2) (2026-11-06)


### Bug Fixes

* **chart:** re-verify the data bundle when a Pod restarts in place ([#53](https://github.com/tekulvw/mag-recomp-server/issues/53)) ([94abfe5](https://github.com/tekulvw/mag-recomp-server/commit/94abfe56039def651fce997d79af139cd8046cd6))

## [0.4.1](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.4.0...chart-v0.4.1) (2026-11-04)


### Bug Fixes

* **gameplay:** rotate evidence logs with time-based retention ([#52](https://github.com/tekulvw/mag-recomp-server/issues/52)) ([d202178](https://github.com/tekulvw/mag-recomp-server/commit/d202178d14ddde656aa0d8aba7670bc1ddceb7a9))

## [0.4.0](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.3.0...chart-v0.4.0) (2026-11-04)


### Features

* bundle deployed server changelogs into in-game NEWS ([#42](https://github.com/tekulvw/mag-recomp-server/issues/42)) ([afb360e](https://github.com/tekulvw/mag-recomp-server/commit/afb360edf93156ad9f3c6842794cfab534ce3ace))

## [0.3.0](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.2.0...chart-v0.3.0) (2026-10-31)


### Features

* **gameplay:** load external bot identities ([#13](https://github.com/tekulvw/mag-recomp-server/issues/13)) ([8b7c003](https://github.com/tekulvw/mag-recomp-server/commit/8b7c0030232a99a79e132990b31b457d16fadda9))

## [0.2.0](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.1.1...chart-v0.2.0) (2026-10-31)


### Features

* use explicit retail catalogs for gameplay and deployment ([#11](https://github.com/tekulvw/mag-recomp-server/issues/11)) ([53495c9](https://github.com/tekulvw/mag-recomp-server/commit/53495c9841891a90bf3535c5770f2619f20860fa))

## [0.1.1](https://github.com/tekulvw/mag-recomp-server/compare/chart-v0.1.0...chart-v0.1.1) (2026-10-31)


### Bug Fixes

* **chart:** inspect listeners without opening protocol connections ([#6](https://github.com/tekulvw/mag-recomp-server/issues/6)) ([b71c1c6](https://github.com/tekulvw/mag-recomp-server/commit/b71c1c6b39f383d1602ba7c7a2a2f1d51fd12345))

## 0.1.0 (2026-10-31)


### Features

* add independently released Helm deployment and optional forwarding DNS ([db56935](https://github.com/tekulvw/mag-recomp-server/commit/db56935611327a7319edca1b67b0f48d569b14df))
* add independently released Helm deployment and optional forwarding DNS ([beb2e0c](https://github.com/tekulvw/mag-recomp-server/commit/beb2e0cbacb518074a1f2967a085be419e4c7a78))
