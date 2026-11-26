using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;

namespace Server.Medius
{
    /// <summary>One gameplay host the operator kill may search, named as the server status names it.</summary>
    public sealed record MagAdminGameplayHost(string WorkerId, LocalDmeHost Host);

    /// <summary>Why an operator kill was not applied. <see cref="None"/> means planned (dry run) or applied.</summary>
    public enum MagAdminKillRefusal
    {
        None, NotFound, InvalidReason, NotOnline, NotInMatch, NotAlive, MatchFinished,
        WorkerUnsupported, WorkerUnavailable, WorkerUnanswered, KillRefused, WorkerRefused
    }

    public sealed class MagAdminKillResult
    {
        public MagAdminKillRefusal Refusal { get; init; }
        /// <summary>Operator-readable explanation of a refusal.</summary>
        public string Message { get; init; }
        public bool Applied { get; init; }
        public string Account { get; init; }
        public string WorkerId { get; init; }
        public uint? WorldId { get; init; }
        public ushort? PlayerId { get; init; }
        /// <summary><c>killed</c> or <c>gave_up</c> once applied; null for a dry run or a refusal.</summary>
        public string Result { get; init; }
        /// <summary>The worker's own code for <see cref="MagAdminKillRefusal.KillRefused"/> or
        /// <see cref="MagAdminKillRefusal.WorkerRefused"/>.</summary>
        public string WorkerReason { get; init; }

        /// <summary>Stable API code for a refusal; null for none.</summary>
        public string Code => Refusal switch
        {
            MagAdminKillRefusal.NotFound => "not_found",
            MagAdminKillRefusal.InvalidReason => "invalid_request",
            MagAdminKillRefusal.NotOnline => "not_online",
            MagAdminKillRefusal.NotInMatch => "not_in_match",
            MagAdminKillRefusal.NotAlive => "not_alive",
            MagAdminKillRefusal.MatchFinished => "match_finished",
            MagAdminKillRefusal.WorkerUnsupported => "worker_unsupported",
            MagAdminKillRefusal.WorkerUnavailable => "worker_unavailable",
            MagAdminKillRefusal.WorkerUnanswered => "worker_unanswered",
            MagAdminKillRefusal.KillRefused => "kill_refused",
            MagAdminKillRefusal.WorkerRefused => "worker_refused",
            _ => null
        };
    }

    /// <summary>
    /// Operator kill (docs/admin-cli.md): find the account's player in a running match and ask its
    /// gameplay worker to apply that player's own KillSelf. Shared by the admin API and, through it,
    /// <c>mag-admin character kill</c>. Only workers that advertise <c>admin_kill_v1</c> are asked.
    /// </summary>
    public static class MagAdminKill
    {
        public const int MaximumReasonLength = 256;
        public const int MaximumOperatorLength = 254;

        /// <summary>Printable ASCII, at most <see cref="MaximumReasonLength"/> characters; empty is no reason.</summary>
        public static bool ValidReason(string reason) =>
            reason == null || (reason.Length <= MaximumReasonLength && reason.All(c => c >= 0x20 && c <= 0x7e));

        public static async Task<MagAdminKillResult> KillAsync(LocalCharacterStore store, IReadOnlyList<MagAdminGameplayHost> hosts,
            string account, bool apply, string reason, string operatorName, string markerDirectory, DateTime now)
        {
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || !store.ReadAll().Any(c => c.Account == account))
                return Refuse(MagAdminKillRefusal.NotFound, account, $"account {account} has no character");
            if (!ValidReason(reason))
                return Refuse(MagAdminKillRefusal.InvalidReason, account, $"the reason must be printable ASCII of at most {MaximumReasonLength} characters");
            if (string.IsNullOrEmpty(reason)) reason = null;
            string actor = operatorName == null ? "" : new string(operatorName.Where(c => c >= 0x20 && c <= 0x7e).Take(MaximumOperatorLength).ToArray());
            if (actor.Length == 0) actor = "unknown";

            // A player is in at most one running match; a stale row elsewhere only reports NotInMatch.
            (MagAdminGameplayHost Host, LocalDmeAdminKillStatus Status, LocalDmeAdminKillTarget Target) found = default;
            foreach (var host in hosts ?? Array.Empty<MagAdminGameplayHost>())
            {
                var (status, candidate) = host.Host.FindAdminKillTarget(account);
                if (status == LocalDmeAdminKillStatus.NotHere) continue;
                if (found.Host == null || Rank(status) > Rank(found.Status)) found = (host, status, candidate);
            }
            if (found.Host == null)
            {
                var active = MagActiveAccounts.Read(markerDirectory, now);
                return active != null && !active.ContainsKey(account)
                    ? Refuse(MagAdminKillRefusal.NotOnline, account, $"{account} is not logged in")
                    : Refuse(MagAdminKillRefusal.NotInMatch, account, $"{account} is not in a match (lobby, queue or results)");
            }
            var target = found.Target;
            MagAdminKillResult Located(MagAdminKillRefusal refusal, string message, bool applied = false, string result = null, string workerReason = null) =>
                new MagAdminKillResult
                {
                    Refusal = refusal, Message = message, Applied = applied, Account = account, WorkerId = found.Host.WorkerId,
                    WorldId = target?.WorldId, PlayerId = target?.PlayerId, Result = result, WorkerReason = workerReason
                };
            switch (found.Status)
            {
                case LocalDmeAdminKillStatus.NotInMatch:
                    return Located(MagAdminKillRefusal.NotInMatch, $"{account} is not a placed player of the match on {found.Host.WorkerId} (joining, results or leaving)");
                case LocalDmeAdminKillStatus.WorkerUnavailable:
                    return Located(MagAdminKillRefusal.WorkerUnavailable, $"the world stream of {found.Host.WorkerId} is down; try again shortly");
                case LocalDmeAdminKillStatus.Unsupported:
                    return Located(MagAdminKillRefusal.WorkerUnsupported,
                        $"gameplay worker {found.Host.WorkerId} does not advertise {GameplayBridgeResponse.AdminKillCapability}; update the worker");
            }
            if (!apply) return Located(MagAdminKillRefusal.None, null);

            var reply = await found.Host.Host.AdminKillAsync(target, actor, reason);
            if (reply == null)
                return Located(MagAdminKillRefusal.WorkerUnavailable, "the kill was not sent (the player left or the worker is busy); nothing changed, try again");
            if (reply.Failure != GameplayBridgeFailure.None || (reply.Ok && reply.AdminKill == null))
                return Located(MagAdminKillRefusal.WorkerUnanswered,
                    $"the worker did not answer ({reply.Failure}); the kill may or may not have applied, check the player before retrying");
            if (!reply.Ok)
                return reply.Error switch
                {
                    "not_in_match" => Located(MagAdminKillRefusal.NotInMatch, $"the worker has no placed player for {account}"),
                    "not_alive" => Located(MagAdminKillRefusal.NotAlive, $"{account} is already dead or waiting for its first spawn"),
                    "match_finished" => Located(MagAdminKillRefusal.MatchFinished, $"the match of {account} has finished"),
                    _ => Located(MagAdminKillRefusal.WorkerRefused, $"the worker refused the kill ({Describe(reply.Error)})", workerReason: Describe(reply.Error))
                };
            if (reply.AdminKill.Result == "refused")
                return Located(MagAdminKillRefusal.KillRefused,
                    $"the worker applied the request but its KillSelf was denied ({reply.AdminKill.Reason})", workerReason: reply.AdminKill.Reason);
            return Located(MagAdminKillRefusal.None, null, true, reply.AdminKill.Result);
        }

        static int Rank(LocalDmeAdminKillStatus status) => status switch
        {
            LocalDmeAdminKillStatus.Ready => 4,
            LocalDmeAdminKillStatus.Unsupported => 3,
            LocalDmeAdminKillStatus.WorkerUnavailable => 2,
            LocalDmeAdminKillStatus.NotInMatch => 1,
            _ => 0
        };

        // The code comes from the worker; keep only a short identifier.
        static string Describe(string code) =>
            code != null && code.Length <= 64 && code.All(c => char.IsAsciiLetterOrDigit(c) || c == '_' || c == '-') ? code : "unspecified";

        static MagAdminKillResult Refuse(MagAdminKillRefusal refusal, string account, string message) =>
            new MagAdminKillResult { Refusal = refusal, Account = account, Message = message };
    }
}
