# Source-only build context; runtime data and credentials are supplied separately.
FROM mcr.microsoft.com/dotnet/sdk:10.0@sha256:e70cdb7f80b0348f5cb85f19a8f670fca061f033d57eed12fa003d58b0e06317 AS build
WORKDIR /src
COPY . .
RUN dotnet restore Horizon.Server.sln \
    && dotnet build Horizon.Server.sln -c Release --no-restore \
    && dotnet restore Tests.MagPlugin/Tests.MagPlugin.csproj \
    && dotnet build Tests.MagPlugin/Tests.MagPlugin.csproj -c Release --no-restore

# Offline synthetic tests only. This target never starts a server or a game.
FROM build AS check
RUN dotnet run --project Tests.MagPlugin/Tests.MagPlugin.csproj -c Release --no-build

FROM check AS publish
RUN dotnet publish Server.Medius/Server.Medius.csproj -c Release --no-restore -o /out/medius \
    && dotnet publish Server.Dme/Server.Dme.csproj -c Release --no-restore -o /out/dme \
    && dotnet publish Server.UniverseInformation/Server.UniverseInformation.csproj -c Release --no-restore -o /out/muis
RUN mkdir -p /out/licenses/package-notices \
    && cp LICENSE /out/licenses/Horizon-MIT.txt \
    && cp HighResolutionTimer/LICENSE /out/licenses/HighResolutionTimer-MIT.txt \
    && cp licenses/*.txt /out/licenses/ \
    && find /root/.nuget/packages -type f \( -iname '*license*.txt' -o -iname '*license*.md' -o -iname '*license*.html' -o -iname '*notice*.txt' -o -name '*.nuspec' \) \
       -exec cp --parents -t /out/licenses/package-notices/ '{}' +

FROM mcr.microsoft.com/dotnet/runtime:10.0@sha256:b89586dc17781f25531909993658aa8161205ae38b8cec8847df4a8221a403d5 AS runtime
COPY --from=publish /out /app
COPY LICENSE /licenses/horizon-LICENSE
COPY HighResolutionTimer/LICENSE /licenses/HighResolutionTimer-LICENSE
WORKDIR /app/medius
ENTRYPOINT ["dotnet", "Server.Medius.dll"]
