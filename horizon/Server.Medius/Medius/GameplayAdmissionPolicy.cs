using System;
using System.Globalization;

namespace Server.Medius
{
    /// <summary>How hard one join tries to reserve a gameplay seat before it gives up.</summary>
    /// <remarks>An admission attempt that the worker never answered (the connection could not
    /// open, or the reserve went unanswered) is retried up to <see cref="Attempts"/> in all. A
    /// reserve that never left Medius is sent again at once; one that was sent is released first
    /// and retried under a new reservation. A definite refusal is never retried. The deadline
    /// bounds each reserve exchange; raising it does not replace the retries.</remarks>
    public sealed class GameplayAdmissionPolicy
    {
        public const string AttemptsVariable = "MAG_GAMEPLAY_ADMISSION_ATTEMPTS";
        public const string DeadlineVariable = "MAG_GAMEPLAY_ADMISSION_DEADLINE_MS";
        public const int DefaultAttempts = 3;
        public const int MaximumAttempts = 5;
        public static readonly TimeSpan MinimumDeadline = TimeSpan.FromMilliseconds(500);
        public static readonly TimeSpan MaximumDeadline = TimeSpan.FromSeconds(10);
        public static GameplayAdmissionPolicy Default { get; } = new(DefaultAttempts, GameplayBridge.RequestDeadline);

        public int Attempts { get; }
        public TimeSpan Deadline { get; }

        public GameplayAdmissionPolicy(int attempts, TimeSpan deadline)
        {
            if (attempts < 1 || attempts > MaximumAttempts)
                throw new ArgumentOutOfRangeException(nameof(attempts), $"Gameplay admission attempts must be 1..{MaximumAttempts}.");
            if (deadline < MinimumDeadline || deadline > MaximumDeadline)
                throw new ArgumentOutOfRangeException(nameof(deadline),
                    $"Gameplay admission deadline must be {MinimumDeadline.TotalMilliseconds}..{MaximumDeadline.TotalMilliseconds} ms.");
            Attempts = attempts;
            Deadline = deadline;
        }

        /// <summary>The operator's policy. Unset variables keep the defaults; an invalid value
        /// stops startup instead of silently falling back.</summary>
        public static GameplayAdmissionPolicy FromEnvironment() =>
            Parse(Environment.GetEnvironmentVariable(AttemptsVariable), Environment.GetEnvironmentVariable(DeadlineVariable));

        public static GameplayAdmissionPolicy Parse(string attempts, string deadlineMilliseconds)
        {
            int count = DefaultAttempts;
            if (!string.IsNullOrEmpty(attempts) &&
                !int.TryParse(attempts, NumberStyles.None, CultureInfo.InvariantCulture, out count))
                throw new ArgumentException($"{AttemptsVariable} must be a whole number of attempts.");
            var deadline = GameplayBridge.RequestDeadline;
            if (!string.IsNullOrEmpty(deadlineMilliseconds))
            {
                if (!int.TryParse(deadlineMilliseconds, NumberStyles.None, CultureInfo.InvariantCulture, out int milliseconds))
                    throw new ArgumentException($"{DeadlineVariable} must be a whole number of milliseconds.");
                deadline = TimeSpan.FromMilliseconds(milliseconds);
            }
            try { return new GameplayAdmissionPolicy(count, deadline); }
            catch (ArgumentOutOfRangeException error)
            {
                throw new ArgumentException($"{AttemptsVariable}/{DeadlineVariable}: {error.Message}", error);
            }
        }
    }
}
