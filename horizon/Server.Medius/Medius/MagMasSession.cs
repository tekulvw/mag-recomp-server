using System;
using System.IO;
using System.Collections.Generic;
using RT.Models;
using RT.Models.MAG;

namespace Server.Medius
{
    /// <summary>Isolated local MAS protocol session; no Medius account or PSN ticket validation.</summary>
    public sealed class MagMasSession
    {
        public enum SessionState { AwaitingHello, ProtocolInfoSent, ProtocolAcknowledged, LocalDevelopmentAuthenticated, CharacterCreated, AccountLoginSent, CharacterDataSent }
        public SessionState State { get; private set; } = SessionState.AwaitingHello;
        public bool IsEstablished => State != SessionState.AwaitingHello;
        public bool DevelopmentAuthEnabled { get; set; } = false;
        public bool CharacterDataEnabled { get; set; }
        public bool AccountLoginEnabled { get; set; }
        public MagLoginRegistry LoginRegistry { get; set; }
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
                case MagHello _ when State == SessionState.AwaitingHello:
                    State = SessionState.ProtocolInfoSent;
                    ObserveActivity(now);
                    return Reply(applicationId, new MagProtocolInfo { ProtocolVersion = Program.Settings.MagProtocolVersion, BuildNumber = 0 });
                case MagProtocolInfo protocol when State == SessionState.ProtocolInfoSent && protocol.ProtocolVersion == Program.Settings.MagProtocolVersion:
                    // Client build differences are nonfatal. Do not ping-pong the acknowledgement.
                    State = SessionState.ProtocolAcknowledged;
                    break;
                case MagNpAuthenticateRequest request when DevelopmentAuthEnabled &&
                    State == SessionState.ProtocolAcknowledged && MagDevelopmentAccountPolicy.Accepts(request):
                    AuthenticatedAccount = MagDevelopmentAccountPolicy.Account(request);
                    State = SessionState.LocalDevelopmentAuthenticated;
                    ObserveActivity(now);
                    return Reply(applicationId, request is MagRetailNpAuthenticateRequest
                        ? new MagRetailNpAuthenticateResponse { TransactionId = request.TransactionId, Success = true }
                        : new MagNpAuthenticateResponse { TransactionId = request.TransactionId, Success = true });
                case MagAccountLoginRequest request when AccountLoginEnabled && DevelopmentAuthEnabled && CharacterStorageEnabled &&
                    (State == SessionState.LocalDevelopmentAuthenticated || State == SessionState.CharacterCreated):
                    MagLoginTicket ticket = null;
                    try
                    {
                        // Trailer scalars are uninitialized in this prototype. Preserve them in the model, but do not use them as credentials.
                        if (request.VersionMajor == 3 && request.VersionMinor == 3 && request.VersionBuild == 1 &&
                            request.DnasSignature == string.Empty && MagDevelopmentAccountPolicy.HasZeroRsa(request) && CharacterStore != null)
                        {
                            foreach (var character in CharacterStore.ReadForAccount(AuthenticatedAccount))
                                if (character.Name == request.AccountName) { ticket = LoginRegistry?.IssueForAccount(AuthenticatedAccount, character.Id, 1, now); if (ticket != null) _selectedCharacterId = character.Id; }
                        }
                        StorageStatus = ticket == null ? "account-login-denied" : "account-login-ticket-issued";
                    }
                    catch (Exception e) when (e is IOException || e is InvalidDataException || e is UnauthorizedAccessException)
                    { StorageStatus = "account-login-storage-error"; }
                    ObserveActivity(now);
                    if (ticket == null) return Reply(applicationId, request is MagRetailAccountLoginRequest
                        ? new MagRetailAccountLoginResponse { TransactionId = request.TransactionId, Result = 0 }
                        : new MagAccountLoginResponse { TransactionId = request.TransactionId, Result = 0 });
                    State = SessionState.AccountLoginSent;
                    return Reply(applicationId, ticket.CreateResponse(request.TransactionId));
                case MagCharacterDataRequest request when CharacterDataEnabled && AccountLoginEnabled && DevelopmentAuthEnabled &&
                    CharacterStorageEnabled && State == SessionState.AccountLoginSent && MagDevelopmentAccountPolicy.HasZeroRsa(request):
                    try
                    {
                        if (CharacterStore != null)
                        {
                            foreach (var character in CharacterStore.ReadForAccount(AuthenticatedAccount))
                            {
                                bool selected = request.QueryKind == 1 ? request.AccountName == character.Name : request.AccountId == character.Id;
                                if (selected && character.Id == _selectedCharacterId)
                                {
                                    var dataReply = Reply(applicationId, new MagCharacterDataResponse { TransactionId = request.TransactionId, RequestId = request.RequestId,
                                        Success = true, Record = character.ToRecord() });
                                    State = SessionState.CharacterDataSent; StorageStatus = "local-character-data-served"; ObserveActivity(now);
                                    return dataReply;
                                }
                            }
                        }
                        StorageStatus = "character-data-identity-unavailable";
                    }
                    catch (Exception e) when (e is IOException || e is InvalidDataException || e is UnauthorizedAccessException)
                    { StorageStatus = "character-data-storage-error"; }
                    // A negative 0239 still requires a full record. Do not invent one for an unavailable identity.
                    break;
                case MagCreateCharacterRequest request when DevelopmentAuthEnabled && CharacterStorageEnabled &&
                    (State == SessionState.LocalDevelopmentAuthenticated || State == SessionState.CharacterCreated):
                    var result = MagNetResult.Error;
                    try
                    {
                        if (CharacterStore == null) throw new IOException("Character store unavailable.");
                        var outcome = CharacterStore.CreateForAccount(request, AuthenticatedAccount); result = outcome.Result; StorageStatus = outcome.Reason;
                        if (result == MagNetResult.Success) State = SessionState.CharacterCreated;
                    }
                    catch (Exception e) when (e is IOException || e is InvalidDataException || e is UnauthorizedAccessException)
                    { StorageStatus = "storage-error-no-success"; }
                    ObserveActivity(now);
                    return Reply(applicationId, new MagCreateCharacterResponse { TransactionId = request.TransactionId, Result = result });
            }
            // Every unimplemented request remains observable only.
            ObserveActivity(now);
            return Array.Empty<RT_MSG_SERVER_PLUGIN_TO_APP>();
        }

        private static IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> Reply(int applicationId, MagMessage message) =>
            new[] { MagMessageCodec.ToPluginMessageForApplication(message, applicationId) };
    }
}
