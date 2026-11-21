using System.IO;
using System.Linq;

namespace Server.Medius
{
    public sealed record GameplayWorkerIdentity
    {
        public string WorkerId { get; }
        public string IncarnationId { get; }

        internal GameplayWorkerIdentity(string workerId, string incarnationId)
        {
            WorkerId = workerId;
            IncarnationId = incarnationId;
        }

        public static bool IsValidIdentifier(string value) =>
            value is { Length: >= 1 and <= 64 } && value.All(c => char.IsAsciiLetterOrDigit(c) || c == '-' || c == '_');
    }

    public sealed class GameplayWorkerIdentityException : IOException
    {
        internal GameplayWorkerIdentityException() : base("Gameplay worker identity verification failed.") { }
    }
}
