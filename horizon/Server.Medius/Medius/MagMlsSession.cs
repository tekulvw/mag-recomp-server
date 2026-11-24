using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using RT.Models;
using RT.Models.MAG;

namespace Server.Medius
{
    public sealed class MagMlsSession
    {
        public enum SessionState { Unbound, Bound, Accepted, TransportReady, ProtocolSent, Ready }
        public SessionState State { get; private set; }
        public bool IsBound => State != SessionState.Unbound;
        public int ApplicationId { get; private set; }
        public uint AccountId { get; private set; }
        public string AuthenticatedAccount { get; private set; }
        public uint CharacterId { get; private set; }
        public LocalCharacterStore CharacterStore { get; set; }
        public string StorageStatus { get; private set; } = "inactive";
        public Action<string, uint> LoadoutCommitted { get; set; }
        public Func<bool> CanDeleteLoadout { get; set; }
        public string LoadoutRefreshStatus { get; private set; } = "inactive";
        public string SessionKey { get; private set; }
        private MagLoginRegistry _registry;
        private string _channelId;
        private DateTime _lastActivity;
        public bool TryBind(RT_MSG_CLIENT_CONNECT_TCP connect, MagLoginRegistry registry, string channelId, DateTime now)
        {
            if (IsBound || registry == null || !registry.TryBind(connect.AccessToken, connect.SessionKey, connect.AppId,
                connect.TargetWorldId, channelId, now, out var ticket)) return false;
            ApplicationId = ticket.ApplicationId; AccountId = ticket.AccountId; AuthenticatedAccount = ticket.AuthenticatedAccount;
            CharacterId = ticket.CharacterId; SessionKey = ticket.SessionKey;
            _registry = registry; _channelId = channelId; State = SessionState.Bound; _lastActivity = now; return true;
        }
        public bool Accept(DateTime now)
        { if (State != SessionState.Bound) return false; State = SessionState.Accepted; ObserveActivity(now); return true; }
        public bool CompleteTransport(DateTime now)
        { if (State != SessionState.Accepted) return false; State = SessionState.TransportReady; ObserveActivity(now); return true; }
        public void ObserveActivity(DateTime now)
        { if (IsBound && _registry.Touch(_channelId, now)) _lastActivity = now; }
        public bool IsExpired(DateTime now) => IsBound && (now - _lastActivity).TotalSeconds >= 120;
        public IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> HandleMany(RT_MSG_CLIENT_APP_TO_PLUGIN envelope, DateTime now)
        {
            if (!IsBound || IsExpired(now) || !envelope.IsValid || envelope.PluginId != 0 ||
                !MagMessageCodec.TryParseClientForApplication(envelope.Payload, ApplicationId, out var message)) return Array.Empty<RT_MSG_SERVER_PLUGIN_TO_APP>();
            if (message is MagHello && State == SessionState.TransportReady)
            {
                State = SessionState.ProtocolSent; ObserveActivity(now);
                return new[] { MagMessageCodec.ToPluginMessageForApplication(new MagProtocolInfo { ProtocolVersion = Program.Settings.MagProtocolVersion, BuildNumber = 0 }, ApplicationId) };
            }
            if (message is MagProtocolInfo protocol && State == SessionState.ProtocolSent && protocol.ProtocolVersion == Program.Settings.MagProtocolVersion)
                State = SessionState.Ready;
            if (message is MagKeepAlive && ApplicationId == 20231 && State == SessionState.Ready)
            {
                ObserveActivity(now);
                return new[] { MagMessageCodec.ToPluginMessageForApplication(new MagKeepAlive(), ApplicationId) };
            }
            if (message is MagTrainingCompletedRequest && ApplicationId == 20231 && State == SessionState.Ready)
            {
                try
                {
                    if (CharacterStore == null || CharacterId == 0 || !MagDevelopmentAccountPolicy.ValidAccount(AuthenticatedAccount))
                        StorageStatus = "training-identity-or-store-unavailable";
                    else
                    {
                        var outcome = CharacterStore.CompleteTrainingForAccount(AuthenticatedAccount, CharacterId);
                        StorageStatus = outcome.Reason;
                        if (outcome.Result == MagNetResult.Success)
                        {
                            ObserveActivity(now);
                            return new[] { MagMessageCodec.ToPluginMessageForApplication(new MagCharacterRecordNotification { Record = outcome.Record }, ApplicationId) };
                        }
                    }
                }
                catch (Exception e) when (e is IOException || e is InvalidDataException || e is UnauthorizedAccessException)
                { StorageStatus = "training-storage-error-no-success"; }
                ObserveActivity(now);
                return Array.Empty<RT_MSG_SERVER_PLUGIN_TO_APP>();
            }
            if (message is MagPurchaseSkillRequest purchase && State == SessionState.Ready)
            {
                var response = new MagPurchaseSkillResponse();
                try
                {
                    if (CharacterStore != null && CharacterId != 0 && MagDevelopmentAccountPolicy.ValidAccount(AuthenticatedAccount))
                    {
                        var outcome = CharacterStore.PurchaseSkillForAccount(AuthenticatedAccount, CharacterId, purchase.SkillId);
                        StorageStatus = outcome.Reason;
                        var character = CharacterStore.ReadForAccount(AuthenticatedAccount).SingleOrDefault(row => row.Id == CharacterId);
                        if (character != null)
                        {
                            response.Record = character.ToRecord();
                            response.Success = outcome.Result == MagNetResult.Success;
                        }
                    }
                    else StorageStatus = "skill-identity-or-store-unavailable";
                }
                catch (Exception e) when (e is IOException || e is InvalidDataException || e is UnauthorizedAccessException)
                { StorageStatus = "skill-storage-error-no-success"; }
                ObserveActivity(now);
                return new[] { MagMessageCodec.ToPluginMessageForApplication(response, ApplicationId) };
            }
            if (message is MagCharacterDataRequest lookup && State == SessionState.Ready)
            {
                // Lobby player cards may describe any player, so the lookup spans the whole store.
                // Every request is answered: a missing character or unreadable store gets a failure
                // carrying the codec's default (cleared) record, because silence leaves the card waiting.
                var response = new MagCharacterDataResponse { TransactionId = lookup.TransactionId, RequestId = lookup.RequestId };
                try
                {
                    var character = CharacterStore == null ? null : FindCharacter(CharacterStore.ReadAll(), lookup);
                    if (character != null)
                    {
                        response.Record = character.ToRecord(); response.Success = true;
                        StorageStatus = "character-data-served";
                    }
                    else StorageStatus = CharacterStore == null ? "character-data-store-unavailable" : "character-data-not-found";
                }
                catch (Exception e) when (e is IOException || e is InvalidDataException || e is UnauthorizedAccessException)
                { StorageStatus = "character-data-storage-error"; }
                ObserveActivity(now);
                return new[] { MagMessageCodec.ToPluginMessageForApplication(response, ApplicationId) };
            }
            if (State == SessionState.Ready && message is (MagLoadoutUpdateRequest or MagRetailLoadoutDeleteRequest))
            {
                LoadoutRefreshStatus = "not-scheduled";
                bool success = false;
                bool deleting = message is MagRetailLoadoutDeleteRequest;
                try
                {
                    if (deleting && CanDeleteLoadout?.Invoke() != true)
                        StorageStatus = "loadout-layout-capability-unavailable";
                    else if (CharacterStore != null && CharacterId != 0 && MagDevelopmentAccountPolicy.ValidAccount(AuthenticatedAccount))
                    {
                        var outcome = message switch
                        {
                            MagLoadoutUpdateRequest update => CharacterStore.SaveLoadoutForAccount(AuthenticatedAccount, CharacterId, update.LoadoutIndex, update.Loadout),
                            MagRetailLoadoutDeleteRequest delete => CharacterStore.DeleteLoadoutForAccount(AuthenticatedAccount, CharacterId, delete.LoadoutIndex),
                            _ => throw new InvalidOperationException("Invalid Armory operation.")
                        };
                        success = outcome.Result == MagNetResult.Success; StorageStatus = outcome.Reason;
                    }
                    else StorageStatus = "loadout-identity-or-store-unavailable";
                }
                catch (Exception e) when (e is IOException || e is InvalidDataException || e is UnauthorizedAccessException)
                { StorageStatus = "loadout-storage-error-no-success"; }
                if (success && LoadoutCommitted != null)
                {
                    // A refresh failure after a durable edit cannot reverse its native response.
                    try { LoadoutCommitted(AuthenticatedAccount, CharacterId); LoadoutRefreshStatus = "notified"; }
                    catch (Exception) { LoadoutRefreshStatus = "notification-failed-after-commit"; }
                }
                ObserveActivity(now);
                MagMessage response = deleting
                    ? new MagRetailLoadoutDeleteResponse { Success = success }
                    : new MagLoadoutUpdateResponse { Success = success };
                return new[] { MagMessageCodec.ToPluginMessageForApplication(response, ApplicationId) };
            }
            ObserveActivity(now); return Array.Empty<RT_MSG_SERVER_PLUGIN_TO_APP>();
        }
        // Kind 2 names the character id (the id players see in a match); kind 1 names a character.
        // A name is the stored name first, else the one character whose display name it is, since
        // the in-match name of a linked character is its display name.
        private static StoredCharacter FindCharacter(IReadOnlyList<StoredCharacter> characters, MagCharacterDataRequest lookup)
        {
            if (lookup.QueryKind == 2) return lookup.AccountId == 0 ? null : characters.FirstOrDefault(row => row.Id == lookup.AccountId);
            if (lookup.QueryKind != 1 || string.IsNullOrEmpty(lookup.AccountName)) return null;
            var stored = characters.FirstOrDefault(row => row.Name == lookup.AccountName);
            if (stored != null) return stored;
            var shown = characters.Where(row => row.DisplayName() == lookup.AccountName).Take(2).ToArray();
            return shown.Length == 1 ? shown[0] : null;
        }
    }
}
