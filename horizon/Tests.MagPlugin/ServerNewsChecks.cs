using System.Text;
using RT.Models;
using RT.Models.MAG;
using Server.Common.Stream;
using Server.Medius;

static class ServerNewsChecks
{
    static void Check(bool condition, string message)
    { if (!condition) throw new Exception(message); }

    public static void Formatting()
    {
        const string medius = "# Changelog\n## [Unreleased]\n* Not deployed yet\n## [1.2.3](https://example.invalid/compare) (2026-11-01)\n### Features\n* **maps:** A café’s \"news\" — fixed\ttext\0 with [details](https://example.invalid/details) ([#19](https://example.invalid/19)) ([abcdef1](https://example.invalid/c)).\n## 1.2.2 (2026-10-30)\n* Historical Medius note\n";
        const string gameplay = "# Changelog\r\n## 2.3.4 (2026-11-02)\r\n### Bug Fixes\r\n* Synthetic spawn fix\r\n## 2.3.3 (2026-10-31)\r\n* Historical gameplay note\r\n";
        var news = MagServerNews.FromChangelogs(medius, gameplay);
        Check(news.Content.Contains("Medius 1.2.3 (2026-11-01)") && news.Content.Contains("Gameplay 2.3.4 (2026-11-02)"), "Both component releases must be visible");
        Check(news.Content.Contains("cafe's \"news\" - fixed text with details"), "Plain ASCII rendering");
        Check(!news.Content.Contains("Historical") && !news.Content.Contains("Not deployed") && !news.Content.Contains("https://") && !news.Content.Contains("abcdef1") && !news.Content.Contains("#19"), "Only current release text without GitHub metadata");
        Check(news.Content.All(c => c == '\n' || c >= ' ' && c <= '~'), "NEWS contains unsupported wire characters");
        Check(news.Timestamp == 1793577600u && MagServerNews.FromChangelogs(medius, gameplay).Timestamp == news.Timestamp, "Stable latest release-date timestamp");
        var longNews = MagServerNews.FromChangelogs("## 1.2.3 (2026-11-01)\n* " + new string('a', 9000), "## 2.3.4 (2026-11-02)\n* " + new string('b', 9000));
        Check(longNews.Content.Length <= 4096 && longNews.Content.Contains("Medius 1.2.3") && longNews.Content.Contains("Gameplay 2.3.4") && longNews.Content.Contains("[More changes omitted]"), "Long notes must preserve both components within the wire bound");
        foreach (int app in new[] { 20230, 20231 })
        {
            var wire = MagMessageCodec.SerializeForApplication(new MagNewsEulaResponse { Content = longNews.Content, Timestamp = longNews.Timestamp, Finished = true }, app);
            Check(MagMessageCodec.TryParseForApplication(wire, app, out var parsed) && parsed is MagNewsEulaResponse response && response.Content == longNews.Content, "Bounded NEWS wire round trip");
        }
        Check(!MagServerNews.FromChangelogs(medius, null).Content.Contains("Gameplay"), "Legacy gameplay must not invent notes");
        Check(MagServerNews.FromChangelogs(null, null).Timestamp == 0, "Missing changelogs must not advertise a new update");
        Check(!string.IsNullOrWhiteSpace(MagServerNews.FromChangelogs(null, null).Content), "Empty build has a readable fallback");
        Check(MagServerNews.FromChangelogs("# Changelog\n## Unreleased\n* Future change", null).Timestamp == 0, "Unreleased changes must not become NEWS");
        try { MagServerNews.FromChangelogs("## 1.2.3 (2026-99-99)\n* Invalid", null); throw new Exception("Invalid release date accepted"); }
        catch (InvalidDataException) { }
    }

    public static void Run()
    {
        string path = Path.Combine(Path.GetTempPath(), "synthetic-news-" + Guid.NewGuid() + ".md");
        string previous = Environment.GetEnvironmentVariable("MAG_GAMEPLAY_NEWS_PATH");
        int previousApplication = Server.Medius.Program.Settings.MagApplicationId;
        File.WriteAllText(path, "# Changelog\n\n## [2.3.4](https://example.invalid/compare) (2026-11-02)\n\n### Bug Fixes\n\n* Fix synthetic spawns ([#12](https://example.invalid/12)) ([abcdef1](https://example.invalid/commit)).\n\n## 2.3.3 (2026-10-31)\n* Older synthetic fix\n");
        try
        {
            Environment.SetEnvironmentVariable("MAG_GAMEPLAY_NEWS_PATH", path);
            Check(MagServerNews.Current.Content.Contains("Gameplay 2.3.4"), "Bundled NEWS loader ignored deployed gameplay notes");
            foreach (int app in new[] { 20230, 20231 })
            {
                Server.Medius.Program.Settings.MagApplicationId = app;
                var session = new MagMapsSession(MagServerNews.FromChangelogs(null, File.ReadAllText(path))) { DevelopmentAuthEnabled = true };
                var now = new DateTime(2026, 11, 2, 12, 0, 0, DateTimeKind.Utc);
                IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> Send(MagMessage message)
                {
                    byte[] payload = MagMessageCodec.SerializeForApplication(message, app);
                    using var stream = new MemoryStream();
                    using (var writer = new BinaryWriter(stream, Encoding.ASCII, true))
                    { writer.Write((ushort)payload.Length); writer.Write((byte)0); writer.Write(payload); }
                    stream.Position = 0;
                    var envelope = new RT_MSG_CLIENT_APP_TO_PLUGIN();
                    using var reader = new MessageReader(stream);
                    envelope.Deserialize(reader);
                    return session.HandleMany(true, app, envelope, now);
                }
                Check(Send(new MagNewsEulaRequest { Language = "english" }).Count == 0, "Unauthenticated NEWS accepted");
                Send(new MagHello());
                if (app == 20230) Send(new MagUniverseListRequest());
                MagNpAuthenticateRequest auth = app == 20231 ? new MagRetailNpAuthenticateRequest() : new MagNpAuthenticateRequest();
                auth.FullTicketSize = auth.TicketChunkSize = 9;
                auth.LastMessage = true;
                Encoding.ASCII.GetBytes("synthetic").CopyTo(auth.TicketChunk, 0);
                Send(auth);
                if (app == 20231) Send(new MagRetailUniverseListRequest());
                Check(session.State == MagMapsSession.SessionState.LocalDevelopmentAuthenticated, "Synthetic MAPS authentication");
                Check(Send(new MagNewsEulaRequest { Language = "french" }).Count == 0, "Unsupported language changed state");
                var replies = Send(new MagNewsEulaRequest { TransactionId = 42, Language = "english" });
                Check(replies.Count == 2 && session.State == MagMapsSession.SessionState.ContentSent, "NEWS/EULA state or count");
                Check(MagMessageCodec.TryParseForApplication(replies[0].Payload, app, out var parsed) && parsed is MagNewsEulaResponse, "NEWS wire response");
                var news = (MagNewsEulaResponse)parsed;
                Check(news.TransactionId == 42 && news.Finished && news.ContentType == MagContentType.News, "NEWS transaction and completion");
                Check(news.Content.Contains("Gameplay 2.3.4") && news.Content.Contains("Fix synthetic spawns"), "Deployed gameplay changelog missing from NEWS");
                Check(!news.Content.Contains("Older synthetic") && !news.Content.Contains("https://"), "Historical notes or raw Markdown leaked into NEWS");
                Check(news.Timestamp == 1793577600u, "NEWS timestamp must come from the release date");
                Check(MagMessageCodec.TryParseForApplication(replies[1].Payload, app, out parsed) && parsed is MagNewsEulaResponse, "EULA wire response");
                var eula = (MagNewsEulaResponse)parsed;
                Check(eula.TransactionId == 42 && eula.Finished && eula.ContentType == MagContentType.Eula && eula.Content == "" && eula.Timestamp == 0, "EULA changed");
                Check(Send(new MagCharacterListRequest()).Count == 1, "NEWS blocked the character list");
            }
        }
        finally
        { Environment.SetEnvironmentVariable("MAG_GAMEPLAY_NEWS_PATH", previous); Server.Medius.Program.Settings.MagApplicationId = previousApplication; File.Delete(path); }
    }
}
