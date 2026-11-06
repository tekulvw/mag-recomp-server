using Server.Medius;

// Synthetic timing checks: a release that Rust answers without acknowledging
// (for example a reservation it already evicted) is retried, then abandoned once
// Rust's idle timeout guarantees the reservation is gone, so the account's
// admission is never held pending forever.
static class RejectedReleaseChecks
{
    static void Check(bool value,string message) {if(!value)throw new Exception(message);}
    public static void Run()
    {
        var first=new DateTime(2026,1,1,0,0,0,DateTimeKind.Utc);
        var retry=LocalDmeHost.NextRejectedReleaseAttempt(first,first);
        Check(retry==first+LocalDmeHost.RejectedReleaseRetryInterval,"First rejection retries after the interval");
        Check(retry!=DateTime.MaxValue,"Rejected release must not wait forever");
        var late=first+LocalDmeHost.RejectedReleaseAbandonAfter-TimeSpan.FromSeconds(1);
        Check(LocalDmeHost.NextRejectedReleaseAttempt(first,late)!=null,"Rejected release kept within the grace");
        Check(LocalDmeHost.NextRejectedReleaseAttempt(first,first+LocalDmeHost.RejectedReleaseAbandonAfter)==null,"Rejected release abandoned at the grace");
        Check(LocalDmeHost.NextRejectedReleaseAttempt(first,first.AddHours(1))==null,"Rejected release abandoned after the grace");
        Check(LocalDmeHost.RejectedReleaseAbandonAfter>TimeSpan.FromSeconds(120),"Grace must exceed Rust's 120 s idle timeout");
        Check(LocalDmeHost.RejectedReleaseRetryInterval<LocalDmeHost.RejectedReleaseAbandonAfter,"Retries happen before abandonment");
    }
}
