using System;
using System.IO;
using System.Diagnostics;
using System.Net.Sockets;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
namespace Server.Medius {
 // One bounded JSONL request per connection. Never include credentials in diagnostics.
 public sealed class GameplayBridge {
  readonly string _host; readonly int _port;
  public GameplayBridge(string host,int port) { if(string.IsNullOrWhiteSpace(host)||port<1||port>65535) throw new ArgumentException("Invalid gameplay endpoint."); _host=host;_port=port; }
  public async Task<bool> RequestAsync(object request) => (await ExchangeAsync(request,false))?.Ok == true;
  public Task<GameplayBridgeResponse> RequestEventAsync(object request) => ExchangeAsync(request,true);
  public Task<GameplayBridgeResponse> RequestProgressionAsync(object request) => ExchangeAsync(request,true);
  public Task<GameplayBridgeResponse> RequestReleaseAsync(object request) => ExchangeAsync(request,true);
  // This connection owns only unsolicited world ticks. Ordinary requests retain
  // their existing request/reply path and are never replayed after a timeout.
  public async Task SubscribeWorldAsync(uint worldId,string requestId,Func<GameplayBridgeResponse,Task> receive,CancellationToken cancellationToken) {
   if(string.IsNullOrEmpty(requestId)||receive==null)throw new ArgumentException("Invalid world subscription.");
   using var socket=new TcpClient();
   using(var connect=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken)) {
    connect.CancelAfter(TimeSpan.FromSeconds(2));await socket.ConnectAsync(_host,_port,connect.Token);
   }
   using var stream=socket.GetStream();
   var body=JsonSerializer.SerializeToUtf8Bytes(new {v=2,request_id=requestId,op="subscribe_world",world_id=worldId});
   await stream.WriteAsync(body,cancellationToken);await stream.WriteAsync(new byte[]{10},cancellationToken);
   // Keep framing bounds and a fresh deadline for each heartbeat/frame. A peer
   // that sends partial bytes forever cannot keep a stale world marked ready.
   var buffer=new byte[8192];int offset=0,available=0;
   while(true) {
    using var deadline=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);deadline.CancelAfter(TimeSpan.FromSeconds(3));
    using var line=new MemoryStream();
    while(true) {
     if(offset==available) {
      available=await stream.ReadAsync(buffer,deadline.Token);offset=0;
      if(available==0)throw new IOException("Gameplay world stream closed.");
     }
     int end=Array.IndexOf(buffer,(byte)10,offset,available-offset);
     int count=(end<0?available:end)-offset;
     if(line.Length+count>GameplayBridgeResponseCodec.MaximumResponseBytes)throw new InvalidDataException("Gameplay world stream frame too large.");
     line.Write(buffer,offset,count);offset+=count;
     if(end>=0){offset++;break;}
    }
    if(!GameplayBridgeResponseCodec.TryParse(line.ToArray(),requestId,true,out var reply)||!reply.Ok)throw new InvalidDataException("Invalid gameplay world stream response.");
    cancellationToken.ThrowIfCancellationRequested();await receive(reply);
   }
  }
  async Task<GameplayBridgeResponse> ExchangeAsync(object request,bool eventReply) {
   var elapsed=Stopwatch.StartNew();
   GameplayBridgeResponse Failed(GameplayBridgeFailure failure)=>GameplayBridgeResponse.Failed(failure,elapsed.ElapsedMilliseconds);
   using var deadline=new CancellationTokenSource(TimeSpan.FromSeconds(2));
   try {
    var body=JsonSerializer.SerializeToUtf8Bytes(request); if(body.Length>65536) return Failed(GameplayBridgeFailure.InvalidRequest);
    using var sent=JsonDocument.Parse(body); string id=sent.RootElement.GetProperty("request_id").GetString();
    var op=sent.RootElement.GetProperty("op").GetString();
    if(sent.RootElement.GetProperty("v").GetInt32()!=2 || (eventReply ? op!="event"&&op!="release"&&op!="progression_committed"&&op!="hello" : op=="event"))return Failed(GameplayBridgeFailure.InvalidRequest);
    using var socket=new TcpClient(); await socket.ConnectAsync(_host,_port,deadline.Token); using var stream=socket.GetStream();
    await stream.WriteAsync(body,deadline.Token); await stream.WriteAsync(new byte[]{10},deadline.Token);
    using var line=new MemoryStream(); var one=new byte[1];
    while(true) { if(await stream.ReadAsync(one,deadline.Token)!=1) return Failed(GameplayBridgeFailure.Unavailable); if(one[0]==10) break; if(line.Length==GameplayBridgeResponseCodec.MaximumResponseBytes) return Failed(GameplayBridgeFailure.MalformedResponse); line.WriteByte(one[0]); }
    if(!GameplayBridgeResponseCodec.TryParse(line.ToArray(),id,eventReply,out var reply))return Failed(GameplayBridgeFailure.MalformedResponse);
    reply.ElapsedMilliseconds=elapsed.ElapsedMilliseconds;return reply;
   } catch(OperationCanceledException) { return Failed(GameplayBridgeFailure.Timeout); }
   catch(Exception e) when(e is IOException || e is SocketException) { return Failed(GameplayBridgeFailure.Unavailable); }
   catch(Exception e) when(e is JsonException || e is InvalidOperationException || e is FormatException) { return Failed(GameplayBridgeFailure.InvalidRequest); }
  }
 }
}
