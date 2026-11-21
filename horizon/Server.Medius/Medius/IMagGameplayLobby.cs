using System;
using System.Threading.Tasks;
using RT.Models.MAG;

namespace Server.Medius
{
    public interface IMagGameplayLobby
    {
        bool UsesFactionMatchmaking { get; }
        MagServerStatusResponse Status(int transactionId);
        void QueueJoin(MagMlsSession session, MagJoinGameRequest request, Action<MagPluginClientMessage> send,
            Func<bool> connected, Action<MagCharacterRecord> notify = null);
        void CancelQueuedJoin(MagMlsSession session);
        void ArmoryLoadoutCommitted(string account, uint character);
        Task<MagJoinGameResponse> JoinAsync(MagMlsSession session, MagJoinGameRequest request, Action<MagCharacterRecord> notify = null);
    }
    public sealed partial class LocalDmeHost : IMagGameplayLobby { }
}
