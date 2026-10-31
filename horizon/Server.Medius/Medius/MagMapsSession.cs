using System;
using System.IO;
using System.Collections.Generic;
using System.Linq;
using RT.Models;
using RT.Models.MAG;

namespace Server.Medius
{
    /// <summary>Local development MAPS session; no Medius account or PSN ticket validation.</summary>
    public sealed class MagMapsSession
    {
        public enum SessionState { AwaitingHello, HelloSent, WaitingForAuthentication, LocalDevelopmentAuthenticated, ContentSent, CharacterListSent, WaitingForUniverse }
        public SessionState State { get; private set; } = SessionState.AwaitingHello;
        public bool IsEstablished => State != SessionState.AwaitingHello;
        public bool DevelopmentAuthEnabled { get; set; } = false;
        public bool CharacterStorageEnabled { get; set; }
        public LocalCharacterStore CharacterStore { get; set; }
        public string StorageStatus { get; private set; } = "inactive";
        public string AuthenticatedAccount { get; private set; }
        private uint _selectedCharacterId;
        private DateTime _lastActivityUtc;

        public void ObserveActivity(DateTime now)
        {
            if (IsEstablished) _lastActivityUtc = now;
        }
        public bool IsExpired(DateTime now, int timeoutSeconds) =>
            IsEstablished && (now - _lastActivityUtc).TotalSeconds > timeoutSeconds;

        public IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> HandleMany(bool enabled, int applicationId,
            RT_MSG_CLIENT_APP_TO_PLUGIN envelope, DateTime now)
        {
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
                case MagRetailNpAuthenticateRequest request when DevelopmentAuthEnabled &&
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
                case MagNpAuthenticateRequest request when DevelopmentAuthEnabled &&
                    State == SessionState.WaitingForAuthentication && MagDevelopmentAccountPolicy.Accepts(request):
                    AuthenticatedAccount = MagDevelopmentAccountPolicy.Account(request);
                    State = SessionState.LocalDevelopmentAuthenticated;
                    ObserveActivity(now);
                    return Reply(applicationId, new MagNpAuthenticateResponse { TransactionId = request.TransactionId, Success = true });
                case MagRetailPatchRequest request when applicationId == 20231 && DevelopmentAuthEnabled &&
                    State == SessionState.LocalDevelopmentAuthenticated && request.HighestVersion == 0 && request.InitialVersion == 0 &&
                    request.BuildString == "ABmagprod370048" && request.Language == "english":
                    // Explicit no-download local policy for the captured retail build; retain authentication state for retries.
                    ObserveActivity(now);
                    return Reply(applicationId, new MagRetailNoPatchResponse());
                case MagNewsEulaRequest request when DevelopmentAuthEnabled &&
                    State == SessionState.LocalDevelopmentAuthenticated && request.Language == "english" &&
                    MagDevelopmentAccountPolicy.HasZeroRsa(request):
                    State = SessionState.ContentSent;
                    ObserveActivity(now);
                    return Reply(applicationId,
                        new MagNewsEulaResponse { TransactionId = request.TransactionId, Finished = true, ContentType = MagContentType.News },
                        new MagNewsEulaResponse { TransactionId = request.TransactionId, Finished = true, ContentType = MagContentType.Eula });
                case MagCharacterListRequest request when DevelopmentAuthEnabled && (State == SessionState.ContentSent ||
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
                            response.Characters = CharacterStore.ReadForAccount(AuthenticatedAccount).Select(c => applicationId == 20231
                                ? MagRetailCharacterSummary.FromSummary(c.ToSummary()) : c.ToSummary()).ToList();
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

        private static MagUniverseListResponse UniverseResponse(int transactionId, bool retail)
        {
            // Retail8D2D0/8CAF0 confirms this field order; runtime acceptance remains separate.
            MagUniverseListResponse response = retail ? new MagRetailUniverseListResponse() : new MagUniverseListResponse();
            response.TransactionId = transactionId; response.UniverseId = 26; response.Name = "Local MAG Test";
            response.AuthDns = "mag.test"; response.AuthIp = "127.0.0.1"; response.AuthPort = 10475;
            response.SvoUrl = Program.Settings.MagSvoUrl; response.Success = true; response.IsLast = true;
            return response;
        }

        private static IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> Reply(int applicationId, params MagMessage[] messages) =>
            messages.Select(message => MagMessageCodec.ToPluginMessageForApplication(message, applicationId)).ToArray();
    }
}
