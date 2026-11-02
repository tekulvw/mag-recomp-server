using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using RT.Models.MAG;

namespace Server.Medius
{
    /// <summary>Immutable release notes from the deployed images; never contacts a release service.</summary>
    public sealed class MagServerNews
    {
        private static readonly Lazy<MagServerNews> Bundled = new Lazy<MagServerNews>(Load);
        private static readonly Regex ReleaseHeading = new Regex(
            @"^## (?:\[(?<version>\d+\.\d+\.\d+)\]\([^\r\n]*\)|(?<version>\d+\.\d+\.\d+)) \((?<date>\d{4}-\d{2}-\d{2})\)\s*$");
        private const int MaximumChangelogBytes = 1024 * 1024;
        public static MagServerNews Current => Bundled.Value;
        public string Content { get; }
        public uint Timestamp { get; }

        private MagServerNews(string content, uint timestamp)
        { Content = content; Timestamp = timestamp; }

        private static MagServerNews Load()
        {
            using var embedded = typeof(MagServerNews).Assembly.GetManifestResourceStream("MagServerNews.MediusChangelog");
            using var reader = embedded == null ? null : new StreamReader(embedded);
            string medius = reader?.ReadToEnd();
            string gameplay = null;
            string path = Environment.GetEnvironmentVariable("MAG_GAMEPLAY_NEWS_PATH");
            // Legacy gameplay images have no notes. The chart supplies an empty volume in that case.
            if (!string.IsNullOrEmpty(path))
            {
                if (!Path.IsPathFullyQualified(path)) throw new InvalidDataException("MAG_GAMEPLAY_NEWS_PATH must be absolute.");
                if (File.Exists(path))
                {
                    using var file = File.OpenRead(path);
                    if (file.Length > MaximumChangelogBytes) throw new InvalidDataException("Gameplay changelog exceeds the NEWS input bound.");
                    using var gameplayReader = new StreamReader(file);
                    gameplay = gameplayReader.ReadToEnd();
                }
            }
            return FromChangelogs(medius, gameplay);
        }

        public static MagServerNews FromChangelogs(string medius, string gameplay)
        {
            var sections = new List<string>();
            uint timestamp = 0;
            foreach (var source in new[] { (Name: "Medius", Text: medius), (Name: "Gameplay", Text: gameplay) })
            {
                if (string.IsNullOrWhiteSpace(source.Text)) continue;
                if (source.Text.Length > MaximumChangelogBytes) throw new InvalidDataException("Changelog exceeds the NEWS input bound.");
                using var lines = new StringReader(source.Text);
                string heading = null;
                var body = new List<string>();
                string line;
                while ((line = lines.ReadLine()) != null)
                {
                    if (heading == null)
                    {
                        var match = ReleaseHeading.Match(line);
                        if (!match.Success) continue;
                        if (!DateTimeOffset.TryParseExact(match.Groups["date"].Value, "yyyy-MM-dd", CultureInfo.InvariantCulture,
                            DateTimeStyles.AssumeUniversal, out var date) || date.ToUnixTimeSeconds() < 0 || date.ToUnixTimeSeconds() > uint.MaxValue)
                            throw new InvalidDataException("Invalid NEWS release date.");
                        heading = source.Name + " " + match.Groups["version"].Value + " (" + match.Groups["date"].Value + ")";
                        timestamp = Math.Max(timestamp, (uint)date.ToUnixTimeSeconds());
                    }
                    else
                    {
                        if (line.StartsWith("## ", StringComparison.Ordinal)) break;
                        string plain = PlainText(line);
                        if (plain.Length != 0 || body.Count != 0 && body[body.Count - 1].Length != 0) body.Add(plain);
                    }
                }
                if (heading != null) sections.Add(heading + "\n" + string.Join("\n", body).Trim());
            }
            if (sections.Count == 0) return new MagServerNews("Server update notes are unavailable for this build.", 0);
            // Reserve space for each component, so a long gameplay release cannot hide Medius (or vice versa).
            int budget = (MagMessageCodec.MaximumStringBytes - (sections.Count - 1) * 2) / sections.Count;
            return new MagServerNews(string.Join("\n\n", sections.Select(section => Bound(section, budget))), timestamp);
        }

        private static string Bound(string text, int budget)
        {
            const string omitted = "\n[More changes omitted]";
            return text.Length <= budget ? text : text.Substring(0, budget - omitted.Length).TrimEnd() + omitted;
        }

        private static string PlainText(string line)
        {
            // Drop issue/commit suffixes before retaining the visible labels of other Markdown links.
            line = Regex.Replace(line, @"\s*\(\[(?:#\d+|[a-fA-F0-9]{7,40})\]\([^)]*\)\)", "");
            line = Regex.Replace(line, @"!?\[([^\]]*)\]\([^)]*\)", "$1");
            line = Regex.Replace(line, @"<[^>]*>", "");
            line = Regex.Replace(line.Trim(), @"^#{1,6}\s+", "");
            line = line.Replace("**", "").Replace("`", "");
            if (line.StartsWith("* ", StringComparison.Ordinal)) line = "- " + line.Substring(2);
            line = line.Replace('\u2018', '\'').Replace('\u2019', '\'').Replace('\u201c', '"').Replace('\u201d', '"')
                .Replace('\u2013', '-').Replace('\u2014', '-').Replace('\u00a0', ' ');
            var ascii = new StringBuilder();
            foreach (char c in line.Normalize(NormalizationForm.FormD))
            {
                if (c >= ' ' && c <= '~') ascii.Append(c);
                else if (c == '\t') ascii.Append(' ');
            }
            return ascii.ToString().Trim();
        }
    }
}
