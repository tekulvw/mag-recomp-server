using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;

namespace Server.Medius
{
    /// <summary>The character store's backing database could not be reached or failed; the operation
    /// is uncertain or not applied, exactly as for a failed file commit, and callers defer it.</summary>
    public sealed class CharacterStoreUnavailableException : IOException
    {
        public CharacterStoreUnavailableException(string message, Exception inner = null) : base(message, inner) { }
    }

    /// <summary>A concurrent writer won a uniqueness race; the operation is rerun from the start.</summary>
    internal sealed class CharacterStoreConflictException : IOException
    {
        public CharacterStoreConflictException(string message, Exception inner = null) : base(message, inner) { }
    }

    public sealed partial class LocalCharacterStore
    {
        /// <summary>What one operation touches: an account's character (and, for creation, any character
        /// already holding the requested name), plus a reward receipt and match row for rewards, or the
        /// account's voice sanctions for moderation.</summary>
        private sealed record StoreScope(string Account, uint CharacterId = 0, string RewardId = null, string MatchKey = null,
            string Name = null, bool CreateDirectory = true, bool VoiceSanctions = false);

        private interface ICharacterBackend
        {
            IReadOnlyList<StoredCharacter> ReadAll();
            IReadOnlyList<StoredCharacter> ReadForAccount(string account);
            /// <summary>Opens an exclusive unit of work. Disposing it without <see cref="StoreSession.Commit"/>
            /// discards every change.</summary>
            StoreSession Open(StoreScope scope);
        }

        /// <summary>
        /// One locked read-modify-write. <see cref="Document"/> holds at least everything in the scope; the
        /// JSON backend loads the whole file, PostgreSQL only the scoped rows. Rules mutate it in place.
        /// </summary>
        private abstract class StoreSession : IDisposable
        {
            public Document Document { get; protected set; }
            /// <summary>Maximum number of characters, or null when unbounded.</summary>
            public abstract int? CharacterLimit { get; }
            public abstract uint NextCharacterId();
            /// <summary>Durably saves the document; <paramref name="touchedProgress"/> names the match row a reward changed.</summary>
            public abstract void Commit(string touchedProgress = null);
            /// <summary>Records the state before an operator change and returns where it was recorded.</summary>
            public abstract string Backup(string backupDirectory, string operation, string account, byte level);
            /// <summary>Durably saves <paramref name="account"/>'s voice sanctions (the session's scope) and
            /// journals the change: PostgreSQL in mag.admin_changes in the same transaction, the JSON store as a
            /// copy of the file before it in <paramref name="backupDirectory"/>. Returns where it was journalled.</summary>
            public abstract string CommitVoiceSanctions(string account, string operation, string before, string after, string backupDirectory);
            public abstract void Dispose();
        }

        private static T Retrying<T>(Func<T> operation)
        {
            for (int attempt = 1; ; attempt++)
            {
                try { return operation(); }
                catch (CharacterStoreConflictException) when (attempt < 3) { }
            }
        }

        private sealed class JsonBackend : ICharacterBackend
        {
            private readonly LocalCharacterStore _store;
            public JsonBackend(LocalCharacterStore store) => _store = store;
            public IReadOnlyList<StoredCharacter> ReadAll() => _store.ReadDocument().Characters;
            public IReadOnlyList<StoredCharacter> ReadForAccount(string account) => ReadAll().Where(c => c.Account == account).ToArray();
            public StoreSession Open(StoreScope scope) => new JsonSession(_store, scope.CreateDirectory);
        }

        /// <summary>The whole file under the lock file, committed by atomic rename (see Commit).</summary>
        private sealed class JsonSession : StoreSession
        {
            private readonly LocalCharacterStore _store;
            private readonly string _path;
            private readonly FileStream _lease;
            public JsonSession(LocalCharacterStore store, bool createDirectory)
            {
                _store = store;
                _path = store.ValidPath();
                if (createDirectory) Directory.CreateDirectory(Path.GetDirectoryName(_path));
                _lease = AcquireLock(_path + ".lock");
                try { Document = store.ReadDocument(); }
                catch { _lease.Dispose(); throw; }
            }
            public override int? CharacterLimit => 128;
            public override uint NextCharacterId() =>
                Document.Characters.Count == 0 ? 1u : checked(Document.Characters.Max(c => c.Id) + 1u);
            public override void Commit(string touchedProgress = null) =>
                _store.Commit(_path, Document.Characters, Document, touchedProgress);
            public override string Backup(string backupDirectory, string operation, string account, byte level)
            {
                byte[] current = File.ReadAllBytes(_path);
                string digest = Convert.ToHexString(System.Security.Cryptography.SHA256.HashData(current)).ToLowerInvariant();
                Directory.CreateDirectory(backupDirectory);
                string backupPath = System.IO.Path.Combine(backupDirectory,
                    $"characters.before-admin-{operation}-{account}-{level}-{digest.Substring(0, 12)}.json");
                using (var backup = new FileStream(backupPath, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                { backup.Write(current); backup.Flush(true); }
                return backupPath;
            }
            public override string CommitVoiceSanctions(string account, string operation, string before, string after, string backupDirectory)
            {
                string journal = null;
                if (File.Exists(_path))
                {
                    byte[] current = File.ReadAllBytes(_path);
                    string digest = Convert.ToHexString(System.Security.Cryptography.SHA256.HashData(current)).ToLowerInvariant();
                    Directory.CreateDirectory(backupDirectory);
                    journal = System.IO.Path.Combine(backupDirectory, $"characters.before-admin-{operation}-{account}-{digest.Substring(0, 12)}.json");
                    // The same contents were already kept under this name by an earlier change.
                    if (!File.Exists(journal))
                        using (var backup = new FileStream(journal, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                        { backup.Write(current); backup.Flush(true); }
                }
                Commit();
                return journal;
            }
            public override void Dispose() => _lease.Dispose();
        }
    }
}
