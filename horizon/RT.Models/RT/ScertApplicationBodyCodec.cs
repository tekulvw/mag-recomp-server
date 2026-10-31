using System;
using System.IO;
using System.Linq;
using Server.Common.Stream;
namespace RT.Models {
 /// <summary>Full typed RT application bodies for bridge observation, including recipient routing.
 /// LIST masks are canonicalized by the existing typed serializer; target membership is unchanged.</summary>
 public static class ScertApplicationBodyCodec {
  public static byte[] Serialize(BaseScertMessage message,int mediusVersion) {
   switch(message) {
    case RT_MSG_CLIENT_APP_SINGLE single when single.Payload != null: break;
    case RT_MSG_CLIENT_APP_LIST list when list.Payload != null && list.Targets != null && list.Targets.All(t=>t>=0&&t<2040): break;
    case RT_MSG_CLIENT_APP_BROADCAST broadcast when broadcast.Payload != null: break;
    default: throw new ArgumentException("Unsupported or invalid RT application body.",nameof(message));
   }
   using var stream=new MemoryStream();
   using(var writer=new MessageWriter(stream,System.Text.Encoding.UTF8,true){MediusVersion=mediusVersion})message.Serialize(writer);
   return stream.ToArray();
  }
 }
}
