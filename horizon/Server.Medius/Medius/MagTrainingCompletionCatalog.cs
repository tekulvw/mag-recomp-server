using System;
using System.IO;
using System.Text.Json;

namespace Server.Medius
{
    public sealed class MagTrainingCompletionCatalog
    {
        public byte CompletedFlagMask { get; private set; }
        public int ExperienceBonus { get; private set; }
        public MagProgressionCatalog Progression { get; private set; }
        public static MagTrainingCompletionCatalog Load(string path)
        {
            var progression = MagProgressionCatalog.Load(path, 20231);
            try
            {
                using var document = JsonDocument.Parse(File.ReadAllBytes(path));
                byte mask = document.RootElement.GetProperty("training_completion").GetProperty("completed_flag_mask").GetByte();
                int bonus = document.RootElement.GetProperty("table").GetProperty("Training_ExperienceBonus").GetInt32();
                if (mask == 0 || (mask & (mask - 1)) != 0 || bonus <= 0)
                    throw new InvalidDataException("Invalid Training completion rule.");
                return new MagTrainingCompletionCatalog { CompletedFlagMask = mask, ExperienceBonus = bonus, Progression = progression };
            }
            catch (Exception e) when (e is JsonException || e is System.Collections.Generic.KeyNotFoundException || e is InvalidOperationException || e is FormatException || e is OverflowException)
            { throw new InvalidDataException("Invalid Training completion catalog.", e); }
        }
    }
}
