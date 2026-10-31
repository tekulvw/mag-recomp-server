# Third-party source notices

The root MIT license covers new project contributions. It does not replace
upstream notices or grant any rights to the game or its resources.

| Component | Origin | Notice |
| --- | --- | --- |
| Horizon server | [Horizon-Private-Server/horizon-server](https://github.com/Horizon-Private-Server/horizon-server), import based on `b1a0d0398f58b2d2b72ffc122053e788e794185a` | [horizon/LICENSE](horizon/LICENSE), copyright 2020 Daniel Gerendasy, MIT |
| HighResolutionTimer | [HakanL/Haukcode.HighResolutionTimer](https://github.com/HakanL/Haukcode.HighResolutionTimer) | [horizon/HighResolutionTimer/LICENSE](horizon/HighResolutionTimer/LICENSE), copyright 2020 Hakan Lindestaf, MIT |
| RecastNavigation / Detour | [recastnavigation/recastnavigation](https://github.com/recastnavigation/recastnavigation), v1.6.0, `6dc1667f580357e8a2154c28b7867bea7e8ad3a7` | [License.txt](gameplay/vendor/recastnavigation/License.txt), copyright 2009 Mikko Mononen, zlib |
| DotNetty-derived pipeline decoders | [Azure/DotNetty](https://github.com/Azure/DotNetty) | Microsoft source headers retained; [MIT license](licenses/DotNetty-MIT.txt) |
| Bouncy Castle-derived PS2 RC4 implementation | [RC4Engine source](https://github.com/bcgit/bc-csharp/blob/f18a2dbbc2c1b4277e24a2e51f09cac02eedf1f5/crypto/src/crypto/engines/RC4Engine.cs) | [Bouncy Castle notice](licenses/Bouncy-Castle.txt), from the pinned source tree |

The modified Horizon tree includes the MAG protocol integration, external
resource loading, generated development-key defaults, external certificate
template inputs and public synthetic checks. It is not an unmodified upstream
distribution. Recast/Detour source remains unmodified; the project's C++ bridge
is outside the vendor directory.

Inherited utility snippet implementations in `horizon/Server.Common/Utils.cs`
were replaced during this extraction with current .NET library operations and
independently written boundary handling. The inherited origins were Stack
Overflow answers credited to Servy and Ash, the TimeoutAfter discussion at
`https://stackoverflow.com/a/22078975`, and a C# Corner public-IP scraping example.
The copied grouping, minimum-selection, timeout and HTTP scraping bodies are
not part of this export. Public-IP discovery now requires explicit configuration.

Cargo and NuGet dependencies are named in their manifests and retain their own
licenses. Distributing compiled artifacts also requires retaining applicable
dependency notices. No upstream author's name implies endorsement of this
project.
