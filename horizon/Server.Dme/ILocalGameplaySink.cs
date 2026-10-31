using RT.Models; using Server.Dme.Models;
namespace Server.Dme {
 // Explicit opt-in extension. Null preserves the stock DME path.
 public interface ILocalGameplaySink {
  bool TryBind(ClientObject client, RT_MSG_CLIENT_CONNECT_TCP_AUX_UDP connect);
  bool CanUse(ClientObject client);
  void Connected(ClientObject client);
  void Event(ClientObject client,string transport,byte rtType,byte? pluginId,byte[] payload);
 }
}
