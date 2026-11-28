using System;
using System.IO;
using System.Collections.Generic;
using System.Linq;
using RT.Models;
using RT.Models.MAG;

namespace Server.Medius
{
    /// <summary>Local development MAPS session; no Medius account or PSN ticket validation.</summary>
    public sealed class MagMapsSession : IMagPortalSession
    {
        public enum SessionState { AwaitingHello, HelloSent, WaitingForAuthentication, LocalDevelopmentAuthenticated, ContentSent, CharacterListSent, WaitingForUniverse }
        public SessionState State { get; private set; } = SessionState.AwaitingHello;
        public bool IsEstablished => State != SessionState.AwaitingHello;
        public bool DevelopmentAuthEnabled { get; set; } = false;
        public bool CharacterStorageEnabled { get; set; }
        public LocalCharacterStore CharacterStore { get; set; }
        public string StorageStatus { get; private set; } = "inactive";
        public string AuthenticatedAccount { get; private set; }
        /// <summary>Portal tickets in required mode; null keeps the development identity policy.</summary>
        public MagPortalTicketPolicy PortalTickets { get; set; }
        /// <summary>Portal account ID of a ticket-authenticated session; 0 otherwise.</summary>
        public uint PortalAccountId { get; private set; }
        public string AuthenticatedPublicName { get; private set; }
        public string PortalTicketStatus { get; private set; } = "inactive";
        /// <summary>Staff log line when the last message refused a portal ticket; null otherwise.</summary>
        public string PortalTicketRefusal { get; private set; }
        private readonly MagPortalTicketAssembly _portalTicket = new MagPortalTicketAssembly();
        /// <summary>Idle limit for portal account liveness; MAS.cs sets the application's client timeout.</summary>
        public int PortalLiveTimeoutSeconds { get; set; } = 30;
        private bool _portalSessionEnded;
        public bool IsPortalSessionLive(DateTime now) =>
            !_portalSessionEnded && !IsExpired(now, PortalLiveTimeoutSeconds);
        /// <summary>The connection closed: free its portal account for the next sign-in.</summary>
        public void EndPortalSession()
        {
            _portalSessionEnded = true;
            PortalTickets?.Sessions.Release(this);
        }
        // Portal tickets replace the development identity check, not the local account stack behind it.
        private bool AccountAuthEnabled => DevelopmentAuthEnabled || PortalTickets != null;
        private uint _selectedCharacterId;
        private DateTime _lastActivityUtc;
        private readonly MagServerNews _news;

        public MagMapsSession(MagServerNews news = null) => _news = news;

        public void ObserveActivity(DateTime now)
        {
            if (IsEstablished) _lastActivityUtc = now;
        }
        public bool IsExpired(DateTime now, int timeoutSeconds) =>
            IsEstablished && (now - _lastActivityUtc).TotalSeconds > timeoutSeconds;

        public IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> HandleMany(bool enabled, int applicationId,
            RT_MSG_CLIENT_APP_TO_PLUGIN envelope, DateTime now)
        {
            PortalTicketRefusal = null;
            if (!enabled || !MagPluginRouting.IsSelectedApplication(applicationId) || !envelope.IsValid || envelope.PluginId != 0)
                return Array.Empty<RT_MSG_SERVER_PLUGIN_TO_APP>();
            if (!MagMessageCodec.TryParseClientForApplication(envelope.Payload, applicationId, out var message))
            {
                ObserveActivity(now);
                return Array.Empty<RT_MSG_SERVER_PLUGIN_TO_APP>();
            }
            switch (message)
            {
                case MagKeepAlive _ when applicationId == 20231:
                    ObserveActivity(now);
                    return Reply(applicationId, new MagKeepAlive());
                case MagHello _ when State == SessionState.AwaitingHello:
                    State = SessionState.HelloSent;
                    ObserveActivity(now);
                    // Empty/zero auxiliary values are explicit local policy awaiting retail runtime validation.
                    return Reply(applicationId, applicationId == 20231
                        ? new MagRetailMapsHelloResponse { Success = true, Online = true, FactionMask = 7, AuxiliaryString = string.Empty, AuxiliaryValue = 0 }
                        : new MagMapsHelloResponse { Success = true, Online = true, FactionMask = 7 });
                case MagRetailNpAuthenticateRequest request when PortalTickets != null && State == SessionState.HelloSent:
                    if (!PortalAuthenticate(request, now, out var retailRefusal)) return Refuse(applicationId, retailRefusal);
                    State = SessionState.WaitingForUniverse;
                    return Reply(applicationId, new MagRetailNpAuthenticateResponse { TransactionId = request.TransactionId, Success = true });
                case MagRetailNpAuthenticateRequest request when PortalTickets == null && DevelopmentAuthEnabled &&
                    State == SessionState.HelloSent && MagDevelopmentAccountPolicy.Accepts(request):
                    AuthenticatedAccount = MagDevelopmentAccountPolicy.Account(request);
                    State = SessionState.WaitingForUniverse;
                    ObserveActivity(now);
                    return Reply(applicationId, new MagRetailNpAuthenticateResponse { TransactionId = request.TransactionId, Success = true });
                case MagRetailUniverseListRequest request when State == SessionState.WaitingForUniverse:
                    State = SessionState.LocalDevelopmentAuthenticated;
                    ObserveActivity(now);
                    return Reply(applicationId, UniverseResponse(request.TransactionId, true));
                case MagUniverseListRequest request when applicationId == 20230 && State == SessionState.HelloSent:
                    State = SessionState.WaitingForAuthentication;
                    ObserveActivity(now);
                    return Reply(applicationId, UniverseResponse(request.TransactionId, false));
                case MagNpAuthenticateRequest request when PortalTickets != null && State == SessionState.WaitingForAuthentication:
                    if (!PortalAuthenticate(request, now, out var refusal)) return Refuse(applicationId, refusal);
                    State = SessionState.LocalDevelopmentAuthenticated;
                    return Reply(applicationId, new MagNpAuthenticateResponse { TransactionId = request.TransactionId, Success = true });
                case MagNpAuthenticateRequest request when PortalTickets == null && DevelopmentAuthEnabled &&
                    State == SessionState.WaitingForAuthentication && MagDevelopmentAccountPolicy.Accepts(request):
                    AuthenticatedAccount = MagDevelopmentAccountPolicy.Account(request);
                    State = SessionState.LocalDevelopmentAuthenticated;
                    ObserveActivity(now);
                    return Reply(applicationId, new MagNpAuthenticateResponse { TransactionId = request.TransactionId, Success = true });
                case MagRetailPatchRequest request when applicationId == 20231 && AccountAuthEnabled &&
                    State == SessionState.LocalDevelopmentAuthenticated && request.HighestVersion == 0 && request.InitialVersion == 0 &&
                    request.BuildString == "ABmagprod370048" && request.Language == "english":
                    // Explicit no-download local policy for the captured retail build; retain authentication state for retries.
                    ObserveActivity(now);
                    return Reply(applicationId, new MagRetailNoPatchResponse());
                case MagNewsEulaRequest request when AccountAuthEnabled &&
                    State == SessionState.LocalDevelopmentAuthenticated && request.Language == "english" &&
                    MagDevelopmentAccountPolicy.HasZeroRsa(request):
                    State = SessionState.ContentSent;
                    ObserveActivity(now);
                    return Reply(applicationId,
                        new MagNewsEulaResponse { TransactionId = request.TransactionId, Finished = true, ContentType = MagContentType.News,
                            Content = (_news ?? MagServerNews.Current).Content, Timestamp = (_news ?? MagServerNews.Current).Timestamp },
                        new MagNewsEulaResponse { TransactionId = request.TransactionId, Finished = true, ContentType = MagContentType.Eula });
                case MagCharacterListRequest request when AccountAuthEnabled && (State == SessionState.ContentSent ||
                    (CharacterStorageEnabled && State == SessionState.CharacterListSent)) &&
                    MagDevelopmentAccountPolicy.HasZeroRsa(request):
                    State = SessionState.CharacterListSent;
                    ObserveActivity(now);
                    MagCharacterListResponse response = applicationId == 20231 ? new MagRetailCharacterListResponse() : new MagCharacterListResponse();
                    response.TransactionId = request.TransactionId; response.Success = true; response.AllowedCharacters = 1;
                    if (CharacterStorageEnabled)
                    {
                        try
                        {
                            if (CharacterStore == null) throw new IOException("Character store unavailable.");
                            response.AllowedCharacters = LocalCharacterStore.CharacterSlots;
                            response.Characters = SlotList(CharacterStore.ReadForAccount(AuthenticatedAccount)).Select(c => applicationId == 20231
                                ? MagRetailCharacterSummary.FromSummary(c) : c).ToList();
                            response.CharacterCount = (byte)response.Characters.Count; StorageStatus = "local-profile-list-read";
                        }
                        catch (Exception e) when (e is IOException || e is InvalidDataException || e is UnauthorizedAccessException)
                        { response.Success = false; StorageStatus = "storage-error-no-success"; }
                    }
                    return Reply(applicationId, response);
            }
            ObserveActivity(now);
            return Array.Empty<RT_MSG_SERVER_PLUGIN_TO_APP>();
        }

        /// <summary>
        /// The account's characters as list positions: position N is slot N. The client shows the allowed
        /// number of slots and treats a summary with id 0 as an empty one (its own offline emulator lists one
        /// character and two cleared summaries). A slot freed below an occupied one is sent as a cleared
        /// summary so the characters above it keep their places; trailing empty slots are left out, so an
        /// account with characters in its first slots gets exactly the list it always had.
        /// </summary>
        internal static List<MagCharacterSummary> SlotList(IReadOnlyList<StoredCharacter> characters)
        {
            var list = new List<MagCharacterSummary>();
            if (characters.Count == 0) return list;
            int highest = characters.Max(c => c.Slot);
            for (int slot = 0; slot <= highest; slot++)
                list.Add(characters.FirstOrDefault(c => c.Slot == slot)?.ToSummary() ?? ClearedSummary());
            return list;
        }
        /// <summary>An empty slot as the client clears one: no id or name, faction -1, a level-1 commando, an empty kit.</summary>
        internal static MagCharacterSummary ClearedSummary() => new MagCharacterSummary { Faction = 0xff };

        private bool PortalAuthenticate(MagNpAuthenticateRequest request, DateTime now, out MagNpAuthenticateResponse refusal)
        {
            var outcome = PortalTickets.Authenticate(_portalTicket, request, this, now);
            PortalTicketStatus = outcome.Reason.Code();
            PortalTicketRefusal = outcome.RefusalLog();
            refusal = MagPortalTicketPolicy.RefusalReply(request, outcome);
            ObserveActivity(now);
            if (!outcome.Accepted) return false;
            AuthenticatedAccount = outcome.Account; AuthenticatedPublicName = outcome.PublicName; PortalAccountId = outcome.AccountId;
            return true;
        }

        private static IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> Refuse(int applicationId, MagNpAuthenticateResponse refusal) =>
            refusal == null ? Array.Empty<RT_MSG_SERVER_PLUGIN_TO_APP>() : Reply(applicationId, refusal);

        private static MagUniverseListResponse UniverseResponse(int transactionId, bool retail)
        {
            // Retail8D2D0/8CAF0 confirms this field order; runtime acceptance remains separate.
            MagUniverseListResponse response = retail ? new MagRetailUniverseListResponse() : new MagUniverseListResponse();
            response.TransactionId = transactionId; response.UniverseId = 26; response.Name = "Local MAG Test";
            var network = MagNetworkSettings.FromEnvironment();
            response.AuthDns = network.AuthDns; response.AuthIp = network.Address.ToString(); response.AuthPort = network.AuthPort;
            response.SvoUrl = Program.Settings.MagSvoUrl; response.Success = true; response.IsLast = true;
            return response;
        }

        private static IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> Reply(int applicationId, params MagMessage[] messages) =>
            messages.Select(message => MagMessageCodec.ToPluginMessageForApplication(message, applicationId)).ToArray();
    }
}
