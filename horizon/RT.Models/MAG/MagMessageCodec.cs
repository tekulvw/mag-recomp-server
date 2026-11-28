using System;
using System.IO;

namespace RT.Models.MAG
{
    /// <summary>Typed MAG application protocol; all fields are MSB-first, including unaligned fields.</summary>
    public static class MagMessageCodec
    {
        public const int MaximumStringBytes = 4096;
        private const int MaximumPayloadBytes = 10237;
        private const int TicketChunkCapacity = 900;
        // Original retail00457560 passes12 to ReadBits; prototype packed items use10.
        private static int LoadoutItemBits(int applicationId) => applicationId switch
        {
            20230 => 10,
            20231 => 12,
            _ => throw new ArgumentOutOfRangeException(nameof(applicationId), "Unsupported MAG application.")
        };

        public static bool TryParseClient(byte[] payload, out MagMessage message) =>
            TryParseClientForApplication(payload, 20230, out message);

        public static bool TryParseClientForApplication(byte[] payload, int applicationId, out MagMessage message)
        {
            if (TryParseForApplication(payload, applicationId, out message) && (message is MagRetailPartyInvite || message is MagRetailPartyInviteResponse ||
                message is MagRetailPartyLeave || message is MagRetailPartyOpenInvite || message is MagRetailPartyRejoinRequest || message is MagRetailCancelJoinRequest || message is MagTrainingCompletedRequest || message is MagKeepAlive || message is MagJoinGameRequest || message is MagServerStatusRequest || message is MagHello || message is MagProtocolInfo || message is MagAccountLoginRequest || message is MagCharacterDataRequest ||
                message is MagNpAuthenticateRequest || message is MagUniverseListRequest || message is MagRetailPatchRequest || message is MagRetailAccountLogoutRequest ||
                message is MagNewsEulaRequest || message is MagCharacterListRequest || message is MagCreateCharacterRequest || message is MagDeleteCharacterRequest || message is MagLoadoutUpdateRequest || message is MagRetailLoadoutDeleteRequest || message is MagPurchaseSkillRequest || message is MagRetailSkillRespecRequest || message is MagRetailNominationsUpdate ||
                message is MagRetailPlayerOnlineRequest || message is MagRetailPlayerGameInfoRequest)) return true;
            message = null;
            return false;
        }

        public static bool TryParse(byte[] payload, out MagMessage message) =>
            TryParseForApplication(payload, 20230, out message);

        public static bool TryParseForApplication(byte[] payload, int applicationId, out MagMessage message)
        {
            message = null;
            if (applicationId != 20230 && applicationId != 20231) return false;
            if (payload == null || payload.Length < 2 || payload.Length > MaximumPayloadBytes) return false;
            try
            {
                int itemBits = LoadoutItemBits(applicationId);
                var reader = new MagBitReader(payload);
                ushort messageType = reader.ReadUInt16();
                // Retail0030/0031 are account logout, not prototype NP auth. Never fall back for conflicting IDs.
                MagMessage parsed = applicationId == 20231 && ((messageType >= 0x0011 && messageType <= 0x0015) || (messageType >= 0x001a && messageType <= 0x0027) || (messageType >= 0x002b && messageType <= 0x0038) || messageType == 0x01f5 || messageType == 0x01f6)
                    ? messageType switch
                    {
                        0x0012 => new MagRetailJoinGameRequest(),
                        0x0013 => new MagRetailJoinGameResponse(),
                        0x0014 => new MagRetailJoinGameUpdate(),
                        0x0015 => new MagRetailCancelJoinRequest(),
                        0x001a => new MagRetailPartyInvite(),
                        0x001b => new MagRetailPartyInviteResponse(),
                        0x001c => new MagRetailPartyLeave(),
                        0x001f => new MagRetailPartyJoined(),
                        0x0020 => new MagRetailPartyLeft(),
                        0x0021 => new MagRetailPartyRoster(),
                        0x0022 => new MagRetailPartyDecline(),
                        0x0025 => new MagRetailPartyOpenInvite(),
                        0x0026 => new MagRetailPartyRejoinRequest(),
                        0x0027 => new MagRetailPartyRejoinResponse(),
                        0x002d => new MagTrainingCompletedRequest(),
                        0x002e => new MagRetailAccountLoginRequest(),
                        0x002f => new MagRetailAccountLoginResponse(),
                        0x0030 => new MagRetailAccountLogoutRequest(),
                        0x0031 => new MagRetailAccountLogoutResponse(),
                        0x01f5 => new MagRetailPatchRequest(),
                        0x01f6 => new MagRetailNoPatchResponse(),
                        0x0034 => new MagRetailNpAuthenticateRequest(),
                        0x0035 => new MagRetailNpAuthenticateResponse(),
                        0x0036 => new MagRetailMapsHelloResponse(),
                        0x0037 => new MagRetailUniverseListRequest(),
                        0x0038 => new MagRetailUniverseListResponse(),
                        _ => null
                    }
                    : messageType switch
                {
                    0x0010 => new MagGamePluginGreeting(),
                    0x0011 => new MagJoinGameRequest(),
                    0x0012 => new MagJoinGameResponse(),
                    0x024b => new MagServerStatusRequest(),
                    0x024c => new MagServerStatusResponse(),
                    0x0001 => new MagHello(),
                    0x0003 => new MagKeepAlive(),
                    0x0002 => new MagProtocolInfo(),
                    0x002b => new MagAccountLoginRequest(),
                    0x002c => new MagAccountLoginResponse(),
                    0x0031 => new MagNpAuthenticateRequest(),
                    0x0032 => new MagNpAuthenticateResponse(),
                    0x0033 => new MagMapsHelloResponse(),
                    0x0034 => new MagUniverseListRequest(),
                    0x0035 => new MagUniverseListResponse(),
                    0x0250 => new MagPurchaseSkillRequest(),
                    0x0251 => new MagPurchaseSkillResponse(),
                    0x0254 when applicationId == 20231 => new MagRetailSkillRespecRequest(),
                    0x0255 when applicationId == 20231 => new MagRetailSkillRespecResponse(),
                    0x021f => new MagLoadoutUpdateRequest(),
                    0x0220 => new MagLoadoutUpdateResponse(),
                    0x0221 when applicationId == 20231 => new MagRetailLoadoutDeleteRequest(),
                    0x0222 when applicationId == 20231 => new MagRetailLoadoutDeleteResponse(),
                    0x0243 when applicationId == 20231 => new MagRetailNominationsUpdate(),
                    0x022b when applicationId == 20231 => new MagRetailPlayerGameInfoRequest(),
                    0x022c when applicationId == 20231 => new MagRetailPlayerGameInfoResponse(),
                    0x022d when applicationId == 20231 => new MagRetailPlayerOnlineRequest(),
                    0x022e when applicationId == 20231 => new MagRetailPlayerOnlineResponse(),
                    0x0236 => new MagCharacterListRequest(),
                    0x0237 => applicationId == 20231 ? new MagRetailCharacterListResponse() : new MagCharacterListResponse(),
                    0x0238 => new MagCharacterDataRequest(),
                    0x0239 => new MagCharacterDataResponse(),
                    0x024f => new MagCharacterRecordNotification(),
                    0x023a => new MagCreateCharacterRequest(),
                    0x023b => new MagCreateCharacterResponse(),
                    0x023c => new MagDeleteCharacterRequest(),
                    0x023d => new MagDeleteCharacterResponse(),
                    0x023e => new MagNewsEulaRequest(),
                    0x023f => new MagNewsEulaResponse(),
                    _ => null
                };
                if (parsed == null) return false;
                if (parsed is MagPluginClientMessage common) ReadHeader(reader, common);
                switch (parsed)
                {
                    case MagPurchaseSkillRequest purchase:
                        purchase.SkillId = unchecked((sbyte)reader.ReadByte());
                        break;
                    case MagPurchaseSkillResponse purchase:
                        purchase.Success = reader.ReadBoolean(); purchase.Record = ReadRecord(reader, itemBits, applicationId == 20231);
                        break;
                    case MagRetailSkillRespecRequest _:
                        break;
                    case MagRetailSkillRespecResponse respec:
                        respec.Success = reader.ReadBoolean();
                        respec.Record = respec.Success ? ReadRecord(reader, itemBits, true) : new MagCharacterRecord();
                        break;
                    case MagLoadoutUpdateRequest update:
                        update.LoadoutIndex = reader.ReadByte();
                        if (update.LoadoutIndex >= 5) return false;
                        update.Loadout = ReadLoadout(reader, itemBits);
                        break;
                    case MagLoadoutUpdateResponse update:
                        update.Success = reader.ReadBoolean();
                        break;
                    case MagRetailLoadoutDeleteRequest delete:
                        delete.LoadoutIndex = reader.ReadByte();
                        if (delete.LoadoutIndex >= 5) return false;
                        break;
                    case MagRetailLoadoutDeleteResponse delete:
                        delete.Success = reader.ReadBoolean();
                        break;
                    case MagRetailAccountLogoutResponse logout:
                        logout.Success = reader.ReadBoolean();
                        break;
                    case MagRetailNominationsUpdate nominations:
                        nominations.Nominations = reader.ReadByte();
                        break;
                    case MagRetailPlayerOnlineRequest online:
                        online.AccountId = reader.ReadUInt32();
                        break;
                    case MagRetailPlayerOnlineResponse online:
                        online.AccountId = reader.ReadUInt32(); online.Online = reader.ReadBoolean();
                        break;
                    case MagRetailPlayerGameInfoRequest gameInfo:
                        gameInfo.AccountId = reader.ReadUInt32();
                        break;
                    case MagRetailPlayerGameInfoResponse gameInfo:
                        gameInfo.AccountId = reader.ReadUInt32(); gameInfo.GameId = reader.ReadUInt32(); gameInfo.LevelName = reader.ReadString();
                        gameInfo.GameType = reader.ReadByte(); gameInfo.TrailingValue1 = reader.ReadByte(); gameInfo.TrailingValue2 = reader.ReadByte();
                        break;
                    case MagRetailPatchRequest patch:
                        patch.HighestVersion = unchecked((int)reader.ReadUInt32()); patch.InitialVersion = unchecked((int)reader.ReadUInt32());
                        patch.BuildString = reader.ReadString(); patch.Language = reader.ReadString();
                        break;
                    case MagRetailNoPatchResponse patch:
                        if (reader.ReadUInt16() != 0) return false;
                        patch.MaxDownloadRate = unchecked((int)reader.ReadUInt32()); patch.MaxCDNDownloadRate = unchecked((int)reader.ReadUInt32());
                        patch.MaxPeers = unchecked((int)reader.ReadUInt32()); patch.WindowSize = reader.ReadUInt32(); patch.EncryptP2P = reader.ReadByte();
                        patch.MaxUploadRate = unchecked((int)reader.ReadUInt32()); patch.MaxUploadSlots = unchecked((int)reader.ReadUInt32());
                        patch.MaxDownloadSlots = unchecked((int)reader.ReadUInt32()); patch.MinDownloadRate = unchecked((int)reader.ReadUInt32());
                        patch.MaxPeerDownloadRate = unchecked((int)reader.ReadUInt32()); patch.TargetCDNOffloadRate = unchecked((int)reader.ReadUInt32());
                        break;
                    case MagRetailJoinGameRequest join:
                        join.Preference=reader.ReadByte(); join.Value=reader.ReadUInt32(); join.Value1=reader.ReadUInt32();
                        join.ClientType=reader.ReadByte(); join.PartyId=reader.ReadUInt32(); break;
                    case MagJoinGameRequest join:
                        join.Preference=reader.ReadByte(); join.Value=reader.ReadUInt32(); join.ClientType=reader.ReadByte(); break;
                    case MagRetailPartyInvite invite:
                        invite.PartyId = reader.ReadUInt32(); invite.Invitee = reader.ReadUInt32(); invite.Inviter = reader.ReadUInt32();
                        break;
                    case MagRetailPartyInviteResponse answer:
                        answer.Response = reader.ReadByte(); answer.Inviter = reader.ReadUInt32();
                        answer.Responder = reader.ReadUInt32(); answer.PartyId = reader.ReadUInt32();
                        break;
                    case MagRetailPartyDecline decline:
                        decline.Declined = reader.ReadUInt32(); decline.PartyId = reader.ReadUInt32(); decline.Reason = reader.ReadByte();
                        break;
                    case MagRetailPartyOpenInvite open:
                        open.OpenInvite = reader.ReadBoolean(); open.PartyId = reader.ReadUInt32();
                        break;
                    case MagRetailPartyRejoinRequest rejoin:
                        rejoin.PartyId = reader.ReadUInt32();
                        break;
                    case MagRetailPartyRejoinResponse voice:
                        voice.PartyId = reader.ReadUInt32(); voice.MediusWorldUid = reader.ReadUInt32();
                        voice.ConnectionType = reader.ReadUInt16(); voice.AddressType = reader.ReadUInt16();
                        voice.ServerAddress = reader.ReadBytes(16); voice.ServerPort = reader.ReadUInt16(); voice.WorldId = reader.ReadUInt32();
                        voice.AccessKey = reader.ReadBytes(17); voice.Piuid = reader.ReadUInt32();
                        break;
                    case MagRetailPartyLeave leave:
                        leave.PartyId = reader.ReadUInt32();
                        break;
                    case MagRetailPartyJoined joined:
                        joined.PartyId = reader.ReadUInt32();
                        break;
                    case MagRetailPartyLeft left:
                        left.PartyId = reader.ReadUInt32();
                        break;
                    case MagRetailPartyRoster roster:
                        int count = reader.ReadUInt16();
                        if (payload.Length != 11 + count * 4) return false;
                        roster.Members = new uint[count];
                        for (int i = 0; i < count; i++) roster.Members[i] = reader.ReadUInt32();
                        roster.PartyId = reader.ReadUInt32();
                        roster.Status = unchecked((sbyte)reader.ReadByte());
                        roster.GameType = unchecked((sbyte)reader.ReadByte());
                        roster.OpenInvite = reader.ReadBoolean();
                        break;
                    case MagRetailCancelJoinRequest cancel:
                        cancel.PartyId=reader.ReadUInt32();break;
                    case MagRetailJoinGameUpdate update:
                        update.Success=reader.ReadBoolean();update.QueuedType=unchecked((sbyte)reader.ReadByte());
                        for(int i=0;i<12;i++)update.QueueSizes[i]=reader.ReadUInt32();break;
                    case MagJoinGameResponse join:
                        join.Result=reader.ReadByte(); join.MediusWorldUid=reader.ReadUInt32(); join.ConnectionType=reader.ReadUInt16(); join.AddressType=reader.ReadUInt16();
                        join.ServerAddress=reader.ReadBytes(16); join.ServerPort=reader.ReadUInt16(); join.WorldId=reader.ReadUInt32(); join.AccessKey=reader.ReadBytes(17); join.Piuid=reader.ReadUInt32(); break;
                    case MagServerStatusResponse status:
                        for(int i=0;i<12;i++) status.PlayersInGame[i]=reader.ReadUInt32();
                        for(int i=0;i<12;i++) status.ShadowWarScores[i]=reader.ReadUInt32();
                        status.ContractOwners=reader.ReadBytes(9);
                        for(int i=0;i<12;i++) status.QueueSizes[i]=reader.ReadUInt32();
                        for(int i=0;i<4;i++) status.MinimumPlayers[i]=reader.ReadUInt32();
                        for(int i=0;i<12;i++) status.QueueEnabled[i]=reader.ReadBoolean(); break;
                    case MagAccountLoginRequest login:
                        login.VersionMajor = unchecked((int)reader.ReadUInt32()); login.VersionMinor = unchecked((int)reader.ReadUInt32());
                        login.VersionBuild = unchecked((int)reader.ReadUInt32()); login.AccountName = reader.ReadString();
                        login.CharacterEncoding = unchecked((int)reader.ReadUInt32()); login.Language = unchecked((int)reader.ReadUInt32());
                        login.DnasSignatureType = unchecked((int)reader.ReadUInt32()); login.DnasSignatureLength = reader.ReadByte();
                        login.DnasSignature = reader.ReadString();
                        break;
                    case MagAccountLoginResponse login:
                        login.LobbyWorldUid = reader.ReadUInt32(); login.AccountId = reader.ReadUInt32();
                        login.ConnectionType = reader.ReadUInt16(); login.AddressType = reader.ReadUInt16(); login.ServerAddress = reader.ReadBytes(16);
                        login.ServerPort = reader.ReadUInt16(); login.WorldId = reader.ReadUInt32();
                        login.AccessKey = reader.ReadBytes(17); login.SessionKey = reader.ReadBytes(17); login.Result = reader.ReadByte();
                        login.Piuid = reader.ReadUInt32(); login.QueuePosition = reader.ReadUInt32(); login.QueueTimeLeft = reader.ReadUInt32();
                        login.TimestampWord0 = reader.ReadUInt32(); login.TimestampWord1 = reader.ReadUInt32();
                        break;
                    case MagProtocolInfo protocol:
                        protocol.ProtocolVersion = reader.ReadUInt32(); protocol.BuildNumber = reader.ReadUInt32();
                        break;
                    case MagNpAuthenticateRequest request:
                        request.FullTicketSize = reader.ReadUInt16(); request.TicketChunk = reader.ReadBytes(TicketChunkCapacity);
                        request.TicketChunkSize = reader.ReadUInt16(); request.LastMessage = reader.ReadBoolean();
                        if (request.TicketChunkSize > TicketChunkCapacity || request.TicketChunkSize > request.FullTicketSize) return false;
                        break;
                    case MagNpAuthenticateResponse response:
                        response.Success = reader.ReadBoolean();
                        break;
                    case MagMapsHelloResponse hello:
                        hello.Success = reader.ReadBoolean(); hello.Online = reader.ReadBoolean(); hello.FactionMask = reader.ReadUInt32();
                        if (hello is MagRetailMapsHelloResponse retailHello)
                        {
                            retailHello.AuxiliaryString = reader.ReadString();
                            retailHello.AuxiliaryValue = reader.ReadUInt16();
                        }
                        break;
                    case MagUniverseListResponse universe:
                        universe.UniverseId = reader.ReadUInt32(); universe.Name = reader.ReadString();
                        universe.AuthDns = reader.ReadString(); universe.AuthIp = reader.ReadString(); universe.AuthPort = reader.ReadUInt32();
                        universe.SvoUrl = reader.ReadString(); universe.Success = reader.ReadBoolean(); universe.IsLast = reader.ReadBoolean();
                        break;
                    case MagNewsEulaRequest contentRequest:
                        contentRequest.Language = reader.ReadString();
                        break;
                    case MagNewsEulaResponse content:
                        content.Finished = reader.ReadBoolean(); content.ContentType = (MagContentType)reader.ReadByte();
                        if (content.ContentType != MagContentType.News && content.ContentType != MagContentType.Eula) return false;
                        content.Content = reader.ReadString(); content.Timestamp = reader.ReadUInt32();
                        break;
                    case MagCharacterRecordNotification notification:
                        notification.Record = ReadRecord(reader, itemBits, applicationId == 20231);
                        break;
                    case MagCharacterDataResponse data:
                        data.Success = reader.ReadBoolean(); data.RequestId = reader.ReadByte(); data.Record = ReadRecord(reader, itemBits, applicationId == 20231);
                        break;
                    case MagCharacterDataRequest data:
                        data.QueryKind = reader.ReadByte();
                        if (data.QueryKind == 1) data.AccountName = reader.ReadString();
                        else if (data.QueryKind == 2) data.AccountId = reader.ReadUInt32();
                        else return false;
                        data.RequestId = reader.ReadByte();
                        break;
                    case MagCreateCharacterRequest create:
                        create.AccountName = reader.ReadCString(); create.Faction = reader.ReadByte();
                        create.HeadId = reader.ReadByte(); create.VoiceId = reader.ReadByte();
                        create.UniverseId = reader.ReadUInt32(); create.NpId = reader.ReadBytes(36);
                        break;
                    case MagCreateCharacterResponse created:
                        created.Result = (MagNetResult)unchecked((int)reader.ReadUInt32());
                        break;
                    case MagDeleteCharacterRequest delete:
                        delete.CharacterName = reader.ReadString(); delete.CharacterId = reader.ReadUInt32();
                        break;
                    case MagDeleteCharacterResponse deleted:
                        deleted.Success = reader.ReadBoolean();
                        break;
                    case MagCharacterListResponse characters:
                        characters.Success = reader.ReadBoolean(); characters.AllowedCharacters = reader.ReadByte(); characters.CharacterCount = reader.ReadByte();
                        for (int i = 0; i < characters.CharacterCount; i++) characters.Characters.Add(
                            characters is MagRetailCharacterListResponse ? ReadRetailCharacter(reader, itemBits) : ReadCharacter(reader, itemBits));
                        break;
                }
                if (!reader.HasOnlyZeroPadding()) return false;
                message = parsed;
                return true;
            }
            catch (InvalidDataException) { return false; }
        }

        // Legacy callers retain prototype behavior; the explicit retail summary type carries its profile.
        // Shared full-record/update types must use the application's explicit serializer at session boundaries.
        public static byte[] Serialize(MagMessage message) =>
            SerializeForApplication(message, message is MagRetailPartyMessage || message is MagTrainingCompletedRequest || message is MagRetailCharacterListResponse || message is MagRetailAccountLoginRequest || message is MagRetailAccountLoginResponse || message is MagRetailJoinGameRequest || message is MagRetailJoinGameResponse || message is MagRetailLoadoutDeleteRequest or MagRetailLoadoutDeleteResponse || message is MagRetailAccountLogoutRequest or MagRetailAccountLogoutResponse ||
                message is MagRetailPlayerOnlineRequest or MagRetailPlayerOnlineResponse or MagRetailPlayerGameInfoRequest or MagRetailPlayerGameInfoResponse ||
                message is MagRetailSkillRespecRequest or MagRetailSkillRespecResponse ? 20231 : 20230);

        public static byte[] SerializeForApplication(MagMessage message, int applicationId)
        {
            if (message == null) throw new ArgumentNullException(nameof(message));
            int itemBits = LoadoutItemBits(applicationId);
            if (message is MagCharacterListResponse && (message is MagRetailCharacterListResponse) != (applicationId == 20231))
                throw new ArgumentException("Character summary type conflicts with selected application.", nameof(message));
            if ((message is MagAccountLoginRequest && (message is MagRetailAccountLoginRequest) != (applicationId == 20231)) ||
                (message is MagAccountLoginResponse && (message is MagRetailAccountLoginResponse) != (applicationId == 20231)))
                throw new ArgumentException("Account login type conflicts with selected application.", nameof(message));
            if ((message is MagJoinGameRequest && (message is MagRetailJoinGameRequest) != (applicationId == 20231)) ||
                (message is MagJoinGameResponse && (message is MagRetailJoinGameResponse) != (applicationId == 20231)))
                throw new ArgumentException("Join game type conflicts with selected application.", nameof(message));
            if (message is MagTrainingCompletedRequest && applicationId != 20231)
                throw new ArgumentException("Training request is retail-only.", nameof(message));
            if ((message is MagRetailJoinGameUpdate || message is MagRetailCancelJoinRequest) && applicationId != 20231)
                throw new ArgumentException("Queue message is retail-only.",nameof(message));
            if (message is MagRetailPartyMessage && applicationId != 20231)
                throw new ArgumentException("Party messages are retail-only.", nameof(message));
            if (message is (MagRetailSkillRespecRequest or MagRetailSkillRespecResponse) && applicationId != 20231)
                throw new ArgumentException("Skill respec messages are retail-only.", nameof(message));
            if (message is (MagRetailLoadoutDeleteRequest or MagRetailLoadoutDeleteResponse) && applicationId != 20231)
                throw new ArgumentException("Loadout delete messages are retail-only.", nameof(message));
            if (message is (MagRetailAccountLogoutRequest or MagRetailAccountLogoutResponse) && applicationId != 20231)
                throw new ArgumentException("Account logout messages are retail-only.", nameof(message));
            if (message is (MagRetailPlayerOnlineRequest or MagRetailPlayerOnlineResponse or MagRetailPlayerGameInfoRequest or MagRetailPlayerGameInfoResponse) && applicationId != 20231)
                throw new ArgumentException("Player status messages are retail-only.", nameof(message));
            var writer = new MagBitWriter();
            writer.WriteUInt16(message.MessageType);
            if (message is MagPluginClientMessage common) WriteHeader(writer, common);
            switch (message)
            {
                case MagTrainingCompletedRequest _:
                case MagKeepAlive _:
                case MagGamePluginGreeting _:
                case MagRetailGamePluginGreeting _:
                case MagServerStatusRequest _:
                case MagRetailAccountLogoutRequest _:
                    break;
                case MagRetailAccountLogoutResponse logout:
                    writer.WriteBoolean(logout.Success);
                    break;
                case MagRetailPlayerOnlineRequest online:
                    writer.WriteUInt32(online.AccountId);
                    break;
                case MagRetailPlayerOnlineResponse online:
                    writer.WriteUInt32(online.AccountId); writer.WriteBoolean(online.Online);
                    break;
                case MagRetailPlayerGameInfoRequest gameInfo:
                    writer.WriteUInt32(gameInfo.AccountId);
                    break;
                case MagRetailPlayerGameInfoResponse gameInfo:
                    writer.WriteUInt32(gameInfo.AccountId); writer.WriteUInt32(gameInfo.GameId); writer.WriteString(gameInfo.LevelName ?? string.Empty);
                    writer.WriteByte(gameInfo.GameType); writer.WriteByte(gameInfo.TrailingValue1); writer.WriteByte(gameInfo.TrailingValue2);
                    break;
                case MagPurchaseSkillRequest purchase:
                    writer.WriteByte(unchecked((byte)purchase.SkillId));
                    break;
                case MagPurchaseSkillResponse purchase:
                    writer.WriteBoolean(purchase.Success); WriteRecord(writer, purchase.Record, itemBits, applicationId == 20231);
                    break;
                case MagRetailSkillRespecRequest _:
                    break;
                case MagRetailSkillRespecResponse respec:
                    writer.WriteBoolean(respec.Success);
                    if (respec.Success) WriteRecord(writer, respec.Record, itemBits, true);
                    break;
                case MagLoadoutUpdateRequest update:
                    if (update.LoadoutIndex >= 5) throw new ArgumentException("Invalid loadout index.");
                    writer.WriteByte(update.LoadoutIndex); WriteLoadout(writer, update.Loadout, itemBits);
                    break;
                case MagLoadoutUpdateResponse update:
                    writer.WriteBoolean(update.Success);
                    break;
                case MagRetailLoadoutDeleteRequest delete:
                    if (delete.LoadoutIndex >= 5) throw new ArgumentException("Invalid loadout index.");
                    writer.WriteByte(delete.LoadoutIndex);
                    break;
                case MagRetailLoadoutDeleteResponse delete:
                    writer.WriteBoolean(delete.Success);
                    break;
                case MagRetailNominationsUpdate nominations:
                    writer.WriteByte(nominations.Nominations);
                    break;
                case MagRetailPatchRequest patch:
                    writer.WriteUInt32(unchecked((uint)patch.HighestVersion)); writer.WriteUInt32(unchecked((uint)patch.InitialVersion));
                    writer.WriteString(patch.BuildString); writer.WriteString(patch.Language);
                    break;
                case MagRetailNoPatchResponse patch:
                    writer.WriteUInt16(0);
                    writer.WriteUInt32(unchecked((uint)patch.MaxDownloadRate)); writer.WriteUInt32(unchecked((uint)patch.MaxCDNDownloadRate));
                    writer.WriteUInt32(unchecked((uint)patch.MaxPeers)); writer.WriteUInt32(patch.WindowSize); writer.WriteByte(patch.EncryptP2P);
                    writer.WriteUInt32(unchecked((uint)patch.MaxUploadRate)); writer.WriteUInt32(unchecked((uint)patch.MaxUploadSlots));
                    writer.WriteUInt32(unchecked((uint)patch.MaxDownloadSlots)); writer.WriteUInt32(unchecked((uint)patch.MinDownloadRate));
                    writer.WriteUInt32(unchecked((uint)patch.MaxPeerDownloadRate)); writer.WriteUInt32(unchecked((uint)patch.TargetCDNOffloadRate));
                    break;
                case MagRetailJoinGameRequest join:
                    writer.WriteByte(join.Preference); writer.WriteUInt32(join.Value); writer.WriteUInt32(join.Value1);
                    writer.WriteByte(join.ClientType); writer.WriteUInt32(join.PartyId); break;
                case MagJoinGameRequest join:
                    writer.WriteByte(join.Preference); writer.WriteUInt32(join.Value); writer.WriteByte(join.ClientType); break;
                case MagRetailPartyInvite invite:
                    writer.WriteUInt32(invite.PartyId); writer.WriteUInt32(invite.Invitee); writer.WriteUInt32(invite.Inviter);
                    break;
                case MagRetailPartyInviteResponse answer:
                    writer.WriteByte(answer.Response); writer.WriteUInt32(answer.Inviter);
                    writer.WriteUInt32(answer.Responder); writer.WriteUInt32(answer.PartyId);
                    break;
                case MagRetailPartyDecline decline:
                    writer.WriteUInt32(decline.Declined); writer.WriteUInt32(decline.PartyId); writer.WriteByte(decline.Reason);
                    break;
                case MagRetailPartyOpenInvite open:
                    writer.WriteBoolean(open.OpenInvite); writer.WriteUInt32(open.PartyId);
                    break;
                case MagRetailPartyRejoinRequest rejoin:
                    writer.WriteUInt32(rejoin.PartyId);
                    break;
                case MagRetailPartyRejoinResponse voice:
                    if (voice.ServerAddress?.Length != 16 || voice.AccessKey?.Length != 17) throw new ArgumentException("Invalid DME descriptor widths.");
                    writer.WriteUInt32(voice.PartyId); writer.WriteUInt32(voice.MediusWorldUid);
                    writer.WriteUInt16(voice.ConnectionType); writer.WriteUInt16(voice.AddressType);
                    writer.WriteBytes(voice.ServerAddress); writer.WriteUInt16(voice.ServerPort); writer.WriteUInt32(voice.WorldId);
                    writer.WriteBytes(voice.AccessKey); writer.WriteUInt32(voice.Piuid);
                    break;
                case MagRetailPartyLeave leave:
                    writer.WriteUInt32(leave.PartyId);
                    break;
                case MagRetailPartyJoined joined:
                    writer.WriteUInt32(joined.PartyId);
                    break;
                case MagRetailPartyLeft left:
                    writer.WriteUInt32(left.PartyId);
                    break;
                case MagRetailPartyRoster roster:
                    if (roster.Members == null || roster.Members.Length > (MaximumPayloadBytes - 11) / 4)
                        throw new ArgumentException("Party roster exceeds the application message bound.", nameof(message));
                    writer.WriteUInt16((ushort)roster.Members.Length);
                    foreach (var member in roster.Members) writer.WriteUInt32(member);
                    writer.WriteUInt32(roster.PartyId);
                    writer.WriteByte(unchecked((byte)roster.Status));
                    writer.WriteByte(unchecked((byte)roster.GameType));
                    writer.WriteBoolean(roster.OpenInvite);
                    break;
                case MagRetailCancelJoinRequest cancel:
                    writer.WriteUInt32(cancel.PartyId);break;
                case MagRetailJoinGameUpdate update:
                    if(update.QueueSizes==null||update.QueueSizes.Length!=12)throw new ArgumentException("Queue matrix requires12 cells.");
                    writer.WriteBoolean(update.Success);writer.WriteByte(unchecked((byte)update.QueuedType));
                    foreach(var count in update.QueueSizes)writer.WriteUInt32(count);break;
                case MagJoinGameResponse join:
                    if(join.ServerAddress?.Length!=16 || join.AccessKey?.Length!=17) throw new ArgumentException("Invalid DME descriptor widths.");
                    writer.WriteByte(join.Result); writer.WriteUInt32(join.MediusWorldUid); writer.WriteUInt16(join.ConnectionType); writer.WriteUInt16(join.AddressType);
                    writer.WriteBytes(join.ServerAddress); writer.WriteUInt16(join.ServerPort); writer.WriteUInt32(join.WorldId); writer.WriteBytes(join.AccessKey); writer.WriteUInt32(join.Piuid); break;
                case MagServerStatusResponse status:
                    if(status.PlayersInGame?.Length!=12 || status.ShadowWarScores?.Length!=12 || status.ContractOwners?.Length!=9 || status.QueueSizes?.Length!=12 || status.MinimumPlayers?.Length!=4 || status.QueueEnabled?.Length!=12) throw new ArgumentException("Invalid status matrix dimensions.");
                    foreach(var n in status.PlayersInGame) writer.WriteUInt32(n);
                    foreach(var n in status.ShadowWarScores) writer.WriteUInt32(n);
                    writer.WriteBytes(status.ContractOwners);
                    foreach(var n in status.QueueSizes) writer.WriteUInt32(n);
                    foreach(var n in status.MinimumPlayers) writer.WriteUInt32(n);
                    foreach(var n in status.QueueEnabled) writer.WriteBoolean(n); break;
                case MagHello _:
                case MagUniverseListRequest _:
                case MagCharacterListRequest _:
                    break;
                case MagAccountLoginRequest login:
                    writer.WriteUInt32(unchecked((uint)login.VersionMajor)); writer.WriteUInt32(unchecked((uint)login.VersionMinor));
                    writer.WriteUInt32(unchecked((uint)login.VersionBuild)); writer.WriteString(login.AccountName);
                    writer.WriteUInt32(unchecked((uint)login.CharacterEncoding)); writer.WriteUInt32(unchecked((uint)login.Language));
                    writer.WriteUInt32(unchecked((uint)login.DnasSignatureType)); writer.WriteByte(login.DnasSignatureLength); writer.WriteString(login.DnasSignature);
                    break;
                case MagAccountLoginResponse login:
                    if (login.ServerAddress?.Length != 16 || login.AccessKey?.Length != 17 || login.SessionKey?.Length != 17)
                        throw new ArgumentException("MAG login descriptor fixed-field width mismatch.");
                    writer.WriteUInt32(login.LobbyWorldUid); writer.WriteUInt32(login.AccountId);
                    writer.WriteUInt16(login.ConnectionType); writer.WriteUInt16(login.AddressType); writer.WriteBytes(login.ServerAddress);
                    writer.WriteUInt16(login.ServerPort); writer.WriteUInt32(login.WorldId); writer.WriteBytes(login.AccessKey); writer.WriteBytes(login.SessionKey);
                    writer.WriteByte(login.Result); writer.WriteUInt32(login.Piuid); writer.WriteUInt32(login.QueuePosition); writer.WriteUInt32(login.QueueTimeLeft);
                    writer.WriteUInt32(login.TimestampWord0); writer.WriteUInt32(login.TimestampWord1);
                    break;
                case MagProtocolInfo protocol:
                    writer.WriteUInt32(protocol.ProtocolVersion); writer.WriteUInt32(protocol.BuildNumber);
                    break;
                case MagNpAuthenticateRequest request:
                    if (request.TicketChunk == null || request.TicketChunk.Length != TicketChunkCapacity ||
                        request.TicketChunkSize > TicketChunkCapacity || request.TicketChunkSize > request.FullTicketSize)
                        throw new ArgumentException("Invalid fixed NP ticket chunk.", nameof(message));
                    writer.WriteUInt16(request.FullTicketSize); writer.WriteBytes(request.TicketChunk);
                    writer.WriteUInt16(request.TicketChunkSize); writer.WriteBoolean(request.LastMessage);
                    break;
                case MagNpAuthenticateResponse response:
                    writer.WriteBoolean(response.Success);
                    break;
                case MagMapsHelloResponse hello:
                    writer.WriteBoolean(hello.Success); writer.WriteBoolean(hello.Online); writer.WriteUInt32(hello.FactionMask);
                    if (hello is MagRetailMapsHelloResponse retailHello)
                    {
                        writer.WriteString(retailHello.AuxiliaryString);
                        writer.WriteUInt16(retailHello.AuxiliaryValue);
                    }
                    break;
                case MagUniverseListResponse universe:
                    writer.WriteUInt32(universe.UniverseId); writer.WriteString(universe.Name); writer.WriteString(universe.AuthDns);
                    writer.WriteString(universe.AuthIp); writer.WriteUInt32(universe.AuthPort); writer.WriteString(universe.SvoUrl);
                    writer.WriteBoolean(universe.Success); writer.WriteBoolean(universe.IsLast);
                    break;
                case MagNewsEulaRequest contentRequest:
                    writer.WriteString(contentRequest.Language);
                    break;
                case MagNewsEulaResponse content:
                    if (content.ContentType != MagContentType.News && content.ContentType != MagContentType.Eula)
                        throw new ArgumentException("Unsupported MAG content type.", nameof(message));
                    writer.WriteBoolean(content.Finished); writer.WriteByte((byte)content.ContentType);
                    writer.WriteString(content.Content); writer.WriteUInt32(content.Timestamp);
                    break;
                case MagCharacterRecordNotification notification:
                    WriteRecord(writer, notification.Record, itemBits, applicationId == 20231);
                    break;
                case MagCharacterDataResponse data:
                    writer.WriteBoolean(data.Success); writer.WriteByte(data.RequestId); WriteRecord(writer, data.Record, itemBits, applicationId == 20231);
                    break;
                case MagCharacterDataRequest data:
                    writer.WriteByte(data.QueryKind);
                    if (data.QueryKind == 1) writer.WriteString(data.AccountName);
                    else if (data.QueryKind == 2) writer.WriteUInt32(data.AccountId);
                    else throw new ArgumentException("Unsupported character-data selector.");
                    writer.WriteByte(data.RequestId);
                    break;
                case MagCreateCharacterRequest create:
                    if (create.NpId == null || create.NpId.Length != 36) throw new ArgumentException("NP identity must be 36 bytes.");
                    writer.WriteCString(create.AccountName); writer.WriteByte(create.Faction); writer.WriteByte(create.HeadId);
                    writer.WriteByte(create.VoiceId); writer.WriteUInt32(create.UniverseId); writer.WriteBytes(create.NpId);
                    break;
                case MagCreateCharacterResponse created:
                    writer.WriteUInt32(unchecked((uint)(int)created.Result));
                    break;
                case MagDeleteCharacterRequest delete:
                    writer.WriteString(delete.CharacterName ?? string.Empty); writer.WriteUInt32(delete.CharacterId);
                    break;
                case MagDeleteCharacterResponse deleted:
                    writer.WriteBoolean(deleted.Success);
                    break;
                case MagCharacterListResponse characters:
                    if (characters.Characters == null || characters.CharacterCount != characters.Characters.Count) throw new ArgumentException("MAG character count mismatch.", nameof(message));
                    writer.WriteBoolean(characters.Success); writer.WriteByte(characters.AllowedCharacters); writer.WriteByte(characters.CharacterCount);
                    foreach (var character in characters.Characters)
                    {
                        if (characters is MagRetailCharacterListResponse)
                        {
                            if (!(character is MagRetailCharacterSummary retailCharacter)) throw new ArgumentException("Retail character lists require retail summaries.", nameof(message));
                            WriteRetailCharacter(writer, retailCharacter, itemBits);
                        }
                        else
                        {
                            if (character is MagRetailCharacterSummary) throw new ArgumentException("Prototype character lists require prototype summaries.", nameof(message));
                            WriteCharacter(writer, character, itemBits);
                        }
                    }
                    break;
                default:
                    throw new ArgumentException("Unsupported typed MAG message.", nameof(message));
            }
            var payload = writer.ToArray();
            if (payload.Length > MaximumPayloadBytes) throw new ArgumentException("MAG payload exceeds RT buffer limit.", nameof(message));
            return payload;
        }

        public static RT_MSG_SERVER_PLUGIN_TO_APP ToPluginMessage(MagMessage message) =>
            new RT_MSG_SERVER_PLUGIN_TO_APP { PluginId = 0, Payload = Serialize(message) };

        public static RT_MSG_SERVER_PLUGIN_TO_APP ToPluginMessageForApplication(MagMessage message, int applicationId) =>
            new RT_MSG_SERVER_PLUGIN_TO_APP { PluginId = 0, Payload = SerializeForApplication(message, applicationId) };

        private static MagLoadout ReadLoadout(MagBitReader reader, int itemBits)
        {
            var loadout = new MagLoadout { UpdateId = reader.ReadByte() };
            for (int i = 0; i < 22; i++) loadout.Items[i] = reader.ReadBoolean() ? (short)reader.ReadBits(itemBits) : (short)-1;
            return loadout;
        }
        private static void WriteLoadout(MagBitWriter writer, MagLoadout loadout, int itemBits)
        {
            if (loadout?.Items == null || loadout.Items.Length != 22) throw new ArgumentException("MAG loadout requires 22 slots.");
            writer.WriteByte(loadout.UpdateId);
            foreach (short item in loadout.Items)
            {
                if (item < -1 || item >= (1 << itemBits)) throw new ArgumentException("MAG inventory index out of range.");
                writer.WriteBoolean(item != -1); if (item != -1) writer.WriteBits((uint)item, itemBits);
            }
        }
        private static MagCharacterRecord ReadRecord(MagBitReader reader, int itemBits, bool retail)
        {
            var c = new MagCharacterRecord { Name = reader.ReadCString(), AccountId = reader.ReadUInt32(),
                Faction = reader.ReadByte(), SkillSpecialty = reader.ReadByte(), Level = reader.ReadByte(), Experience = reader.ReadUInt32() };
            for (int i = 0; i < 24; i++) c.AvailableItems[i] = reader.ReadUInt32();
            for (int i = 0; i < 5; i++) c.Skills[i] = reader.ReadUInt32();
            c.SkillPoints = reader.ReadByte();
            for (int i = 0; i < 5; i++) c.Loadouts[i] = ReadLoadout(reader, itemBits);
            c.LeadershipLevel = reader.ReadByte(); c.LeadershipPoints = reader.ReadUInt32(); c.TimePlayed = reader.ReadUInt32();
            c.LastZonesMux = reader.ReadUInt32(); c.HeadId = reader.ReadByte(); c.VoiceId = reader.ReadByte();
            c.LeadershipNominations = reader.ReadByte(); c.MatchesSinceLastCommand = reader.ReadByte(); c.RespecCount = reader.ReadByte();
            c.RespecPoints = reader.ReadUInt32(); c.UniverseId = reader.ReadUInt32(); c.FactionHistory = reader.ReadByte();
            c.FactionRespec = reader.ReadBoolean(); c.LastUsedLoadout = reader.ReadByte(); c.NpId = reader.ReadBytes(36);
            if (retail) { c.VeteranLevel = reader.ReadByte(); c.Flags = reader.ReadByte(); }
            return c;
        }
        private static void WriteRecord(MagBitWriter writer, MagCharacterRecord c, int itemBits, bool retail)
        {
            if (c == null || c.AvailableItems?.Length != 24 || c.Skills?.Length != 5 || c.Loadouts?.Length != 5 || c.NpId?.Length != 36)
                throw new ArgumentException("MAG full record fixed-field width mismatch.");
            writer.WriteCString(c.Name); writer.WriteUInt32(c.AccountId); writer.WriteByte(c.Faction); writer.WriteByte(c.SkillSpecialty);
            writer.WriteByte(c.Level); writer.WriteUInt32(c.Experience);
            foreach (uint word in c.AvailableItems) writer.WriteUInt32(word);
            foreach (uint word in c.Skills) writer.WriteUInt32(word);
            writer.WriteByte(c.SkillPoints); foreach (var loadout in c.Loadouts) WriteLoadout(writer, loadout, itemBits);
            writer.WriteByte(c.LeadershipLevel); writer.WriteUInt32(c.LeadershipPoints); writer.WriteUInt32(c.TimePlayed);
            writer.WriteUInt32(c.LastZonesMux); writer.WriteByte(c.HeadId); writer.WriteByte(c.VoiceId);
            writer.WriteByte(c.LeadershipNominations); writer.WriteByte(c.MatchesSinceLastCommand); writer.WriteByte(c.RespecCount);
            writer.WriteUInt32(c.RespecPoints); writer.WriteUInt32(c.UniverseId); writer.WriteByte(c.FactionHistory);
            writer.WriteBoolean(c.FactionRespec); writer.WriteByte(c.LastUsedLoadout); writer.WriteBytes(c.NpId);
            if (retail) { writer.WriteByte(c.VeteranLevel); writer.WriteByte(c.Flags); }
        }
        private static MagCharacterSummary ReadCharacter(MagBitReader reader, int itemBits) => ReadCharacter(reader, new MagCharacterSummary(), itemBits);
        private static MagCharacterSummary ReadCharacter(MagBitReader reader, MagCharacterSummary c, int itemBits)
        {
            c.Name = reader.ReadCString(); c.AccountId = reader.ReadUInt32();
            c.Faction = reader.ReadByte(); c.SkillSpecialty = reader.ReadByte(); c.Level = reader.ReadByte();
            c.Loadout = ReadLoadout(reader, itemBits);
            c.UniverseId = reader.ReadUInt32(); c.FactionHistory = reader.ReadByte(); c.RespecAvailable = reader.ReadBoolean();
            c.HeadId = reader.ReadByte(); c.VoiceId = reader.ReadByte(); return c;
        }
        private static MagRetailCharacterSummary ReadRetailCharacter(MagBitReader reader, int itemBits)
        {
            var c = new MagRetailCharacterSummary(); ReadCharacter(reader, c, itemBits);
            c.VeteranLevel = reader.ReadByte(); return c;
        }
        private static void WriteRetailCharacter(MagBitWriter writer, MagRetailCharacterSummary c, int itemBits)
        {
            WriteCharacter(writer, c, itemBits); writer.WriteByte(c.VeteranLevel);
        }
        private static void WriteCharacter(MagBitWriter writer, MagCharacterSummary c, int itemBits)
        {
            if (c == null || c.Loadout?.Items == null || c.Loadout.Items.Length != 22) throw new ArgumentException("MAG loadout requires 22 slots.");
            writer.WriteCString(c.Name); writer.WriteUInt32(c.AccountId); writer.WriteByte(c.Faction);
            writer.WriteByte(c.SkillSpecialty); writer.WriteByte(c.Level); WriteLoadout(writer, c.Loadout, itemBits);
            writer.WriteUInt32(c.UniverseId); writer.WriteByte(c.FactionHistory); writer.WriteBoolean(c.RespecAvailable);
            writer.WriteByte(c.HeadId); writer.WriteByte(c.VoiceId);
        }
        private static void ReadHeader(MagBitReader reader, MagPluginClientMessage message)
        {
            for (int i = 0; i < message.RsaWords.Length; i++) message.RsaWords[i] = reader.ReadUInt32();
            message.TransactionId = unchecked((int)reader.ReadUInt32());
        }
        private static void WriteHeader(MagBitWriter writer, MagPluginClientMessage message)
        {
            if (message.RsaWords == null || message.RsaWords.Length != 16)
                throw new ArgumentException("MAG common header requires sixteen RSA words.", nameof(message));
            foreach (uint word in message.RsaWords) writer.WriteUInt32(word);
            writer.WriteUInt32(unchecked((uint)message.TransactionId));
        }
    }
}
