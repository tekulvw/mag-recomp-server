# Extraction source provenance

These are project-owned extraction implementations adapted for this source release, covered by the repository license. Existing source attribution is retained in file headers. No native game code, resources, tables, captures, or decompiler exports are included. The table records SHA256 of each precursor **source-code file**, before path/CLI/boundary changes; these digests are provenance identifiers, not bundled inputs.

Retail spawn parsing originated from the project's earlier retail spawn extractor (source revision `549f3b4`) and prototype parser (`58e39f7`). Retail exit helpers retain their original source digest in the file header. The PSARC reader and bounded packfile decoder retain their adaptation notes. This does not grant rights in any external game material.

Changes: relocated imports; explicit source/output CLI paths; removed workstation/evidence defaults, original balance witnesses and selection defaults; removed copied source-evidence outputs and embedded native instruction verifiers. Static-world adapters consume external native proof JSON and validate identity; they do not regenerate or independently verify the supplied analysis. `world.py` retains only pure transform decoding/baking helpers. Native ZRB and instance-linkage readers were moved next to the collision parser. Native resource bytes are read only from an explicit archive. Prototype progression emits its catalog without raw source copies. Native structure constants must be supplied externally. Test inputs are newly invented.

| Public source | Precursor source SHA256 |
| --- | --- |
| `tools/collision/archive.py` | `40f06f70734eb80162ba7a01325489cd0f987ee553333374087d5fc2fad40ea5` |
| `tools/collision/catalog.py` | `88ba94c9d95db469e2bb8e8832fdb7e41b67586feadfeb4f0f8c10748ca50102` |
| `tools/collision/extract-retail.py` | `8faeb3a9f7368b130a8737d50064635f8447e455049b1992b7d0e525aea92ae3` |
| `tools/collision/instance_linkage.py` | `e0be20153b0ea05d902d2625dec0bc25f7543ec4f6bc311823ec39486ccd2964` |
| `tools/collision/packfile.py` | `5d6bd311a3c616d1a3117d199254a5eff392fa932f5a48faaa063c0f37781eb4` |
| `tools/collision/retail_compiled.py` | `973c8795f951f00945c269b846db2e940dd3f57c042bb78fcb6f06d0cfc1988a` |
| `tools/collision/retail_placement.py` | `f26bbff4a2160f74082c056db2cffe7f79a8e719ebe9e1487e2c012ad732c056` |
| `tools/collision/retail_terrain.py` | `83e951f4afac4a3afb5c88c4a9c948206d4293caa22d1680ac1807d081aa54de` |
| `tools/collision/world.py` | `220365eca74286adda74f6867303cef2744db208ce39d8012a50c3da766bc79f` |
| `tools/collision/zrb_records.py` | `39906e3f203effd798fd2a12f7a8b8dfadaf566028c8ed191bd14a5500a53822` |
| `tools/extraction/build-retail-spawn-catalog.py` | `4a05a48b9c41b711572e7612bffe430cc65765e924ae6b8e4598ea989874a271` |
| `tools/extraction/extract-bunker-runtime-catalog.py` | `b99ba5accf2d9980f6d490cefc17904daebccec786d64910d62e273fcf399240` |
| `tools/extraction/extract-character-grants.py` | `7bf31ebcfb552b9d81bb6d6fdb7e278c9bc9590ccd5b51f25b643a6eda22cf22` |
| `tools/extraction/extract-item-catalog.py` | `15c4d9088f29504f9f9b0bde52b01a1fe13a26f02eee31d69e0ca038ecd57246` |
| `tools/extraction/extract-loadout-defaults.py` | `f6a89bfd7a6f4b21a7c833a7cba9e7194de50c8dc3f0215080d9dd2c90aedb09` |
| `tools/extraction/extract-progression-catalog.py` | `4a339dc334a37e8d69a71170b08aeaf2c9ab38a57ec333c89e42937553b9abc8` |
| `tools/extraction/extract-retail-armory.py` | `0aa37d39b9144b99d35498f7862a1911afa87d019410edcd90c8be615540b79d` |
| `tools/extraction/extract-retail-catalogs.py` | `30253fac8a216cc4ffa97ce7a2e7229b3483f402b5229f980c18d2dcbc9ffbf5` |
| `tools/extraction/extract-retail-maps.py` | `b9636fd7990da1d4409c21f79d8d38dfb7e1ed59c41272ae19c48fb3066de0cc` |
| `tools/extraction/extract-retail-mediation-profile.py` | `d286872a470fef0d619e086f63802f7e59724ccd76c58d572a55bc8f0b564ddd` |
| `tools/extraction/extract-retail-progression-catalog.py` | `ea2c8c52ddfcd6b50893331feedf92c17830c4e1f3fac469647bb36ebc320722` |
| `tools/extraction/extract-retail-rules.py` | `c51b9cd0c48a3bb65e9c462ed8c21d454671e9c117c37228d1e72d9ccb587b36` |
| `tools/extraction/extract-retail-spawns.py` | `70ada24256f375b4c0382e864984a9d9f1841cd5c3d34196764b0fe28ca5b614` |
| `tools/extraction/extract-skill-catalog.py` | `ea1d09096dcffd88a41319c06975f880c11bde79e3e86c1b826de68349640847` |
| `tools/extraction/project-retail-abilities.py` | `7723f6e2079faafcc965148ea4119a6101689304878d70430ef9edbde678b12f` |
| `tools/extraction/project-retail-leadership-passives.py` | `5edebe8a18d9ce816ae54cac1a06014fb3dbebc6c4287d0a89ff68277a1f2bd1` |
| `tools/extraction/project-retail-leadership-role-policy.py` | `96cf34c282ef265d71824c69578b85f227179aab15e15a6f5a80fe2148cb5413` |
| `tools/extraction/project-retail-objective-observer.py` | `d5e3fdb3d8b297e5663f084231a77474b84c61c24c4a724677c6ded504059025` |
| `tools/extraction/project-retail-objective-runtime.py` | `cd89b47e8504d883af4eb689e27d36eaf71e70ce5b988504bde3b439e9faeb82` |
| `tools/extraction/project-retail-sabotage-objectives.py` | `0671785ab708ec60fcdfb9ca8a90f2638d455dd37575d8cf1ddb8fc68619e626` |
| `tools/extraction/retail_map_helpers.py` | `ca99ab6e92582708d82a3f501e01e1abe791cf3ee91be35c6f03b621353292cc` |
| `tools/navmesh/prepare.py` | `94a670d718a70ce4152220070a18345d23b56e42bbe1b45f188fe6d6b39ff2db` |

## Additional native projectors

These project-owned adaptations require explicit paths and externally qualified proof where stated in [native-projectors.md](../../docs/native-projectors.md). Embedded native instruction tables, structure threshold defaults, equipment selection lists and faction display/map tables are omitted. The falling extractor emits scalar configuration only.

| Public source | Precursor source SHA256 |
| --- | --- |
| `tools/extraction/extract-structure-catalog.py` | `8636a812b4cccbf6c6f1d5d3394decbfa2d26d5bb1cce4844be86e66363fc45f` |
| `tools/extraction/project-character-blast-rules.py` | `5bd66a3f8446594083ee091727646c4f13d44fd9b857ad9170fb6ce1b1324f63` |
| `tools/extraction/project-retail-combat.py` | `d8f9f948f1878a8107cb99b0d1521834ce6bd9bb8e5dad10784f6f155a79da19` |
| `tools/extraction/project-retail-bootstrap.py` | `d92472b0677be0d5e76b1dbfcf87e806c6b9095d1a203494a387862fc0b89141` |
| `tools/extraction/compare-native-item-catalog.py` | `6170b28f016ecb0ba4bd44e81467eb23d82755f0d0fab863ccee2a90405feed4` |
| `tools/extraction/extract-retail-falling-damage.py` | `4e21d4e9c015b8985ba0964d9909269d5fe605ff5e0da075c19899988cb613ed` |
| `tools/extraction/extract-retail-skill-purchases.py` | `68b24ec9adb0a707f5b6cad03fcb1c2441f55e42fe38d47f1a9d3f2715f25176` |
