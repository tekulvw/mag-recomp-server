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
  readonly string _expectedWorkerId;
  GameplayWorkerIdentity _workerIdentity;
  int _workerIdentityFault;
  public GameplayWorkerIdentity WorkerIdentity => Volatile.Read(ref _workerIdentity);
  public bool HasWorkerIdentityFault => Volatile.Read(ref _workerIdentityFault)!=0;
  public GameplayBridge(string host,int port) { if(string.IsNullOrWhiteSpace(host)||port<1||port>65535) throw new ArgumentException("Invalid gameplay endpoint."); _host=host;_port=port; }
  public GameplayBridge(string host,int port,string expectedWorkerId) : this(host,port) {
   if(!GameplayWorkerIdentity.IsValidIdentifier(expectedWorkerId))throw new ArgumentException("Invalid gameplay worker ID.",nameof(expectedWorkerId));
   _expectedWorkerId=expectedWorkerId;
  }
  public Task<GameplayBridgeResponse> RequestControlAsync(object request) => ExchangeAsync(request,false);
  public async Task<bool> RequestAsync(object request) => (await ExchangeAsync(request,false))?.Ok == true;
  public Task<GameplayBridgeResponse> RequestHelloAsync(object request) => ExchangeAsync(request,true);
  public Task<GameplayBridgeResponse> RequestLoadoutSelectionSettlementAsync(object request) => ExchangeAsync(request,true);
  public Task<GameplayBridgeResponse> RequestEventAsync(object request) => ExchangeAsync(request,true);
  public Task<GameplayBridgeResponse> RequestProgressionAsync(object request) => ExchangeAsync(request,true);
  public Task<GameplayBridgeResponse> RequestProgressionRefreshAsync(object request) => ExchangeAsync(request,true);
  public Task<GameplayBridgeResponse> RequestReleaseAsync(object request) => ExchangeAsync(request,true);
  // This connection owns only unsolicited world ticks. Ordinary requests retain
  // their existing request/reply path and are never replayed after a timeout.
  public async Task SubscribeWorldAsync(uint worldId,string requestId,Func<GameplayBridgeResponse,Task> receive,CancellationToken cancellationToken) {
   if(string.IsNullOrEmpty(requestId)||receive==null)throw new ArgumentException("Invalid world subscription.");
   var identity=WorkerIdentity;
   if(_expectedWorkerId!=null&&(identity==null||HasWorkerIdentityFault))throw new GameplayWorkerIdentityException();
   var request=new {v=2,request_id=requestId,op="subscribe_world",world_id=worldId};
   var body=identity==null?JsonSerializer.SerializeToUtf8Bytes(request):AttachWorkerIdentity(JsonSerializer.SerializeToElement(request),identity);
   using var socket=new TcpClient();
   using(var connect=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken)) {
    connect.CancelAfter(TimeSpan.FromSeconds(2));await socket.ConnectAsync(_host,_port,connect.Token);
   }
   using var stream=socket.GetStream();
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
    if(!GameplayBridgeResponseCodec.TryParse(line.ToArray(),requestId,true,out var reply))throw new InvalidDataException("Invalid gameplay world stream response.");
    if(identity!=null&&!VerifyWorkerReply(reply,false))throw new GameplayWorkerIdentityException();
    if(!reply.Ok)throw new InvalidDataException("Invalid gameplay world stream response.");
    cancellationToken.ThrowIfCancellationRequested();await receive(reply);
   }
  }
  static byte[] AttachWorkerIdentity(JsonElement request,GameplayWorkerIdentity identity) {
   using var output=new MemoryStream();
   using(var writer=new Utf8JsonWriter(output)) {
    writer.WriteStartObject();
    foreach(var property in request.EnumerateObject())property.WriteTo(writer);
    writer.WriteStartObject("worker_identity");
    writer.WriteString("worker_id",identity.WorkerId);
    writer.WriteString("incarnation_id",identity.IncarnationId);
    writer.WriteEndObject();
    writer.WriteEndObject();
   }
   return output.ToArray();
  }
  bool VerifyWorkerReply(GameplayBridgeResponse reply,bool hello) {
   if(HasWorkerIdentityFault)return false;
   var received=reply.WorkerIdentity;
   if(received?.WorkerId!=_expectedWorkerId||
    reply.Error is "worker_identity_required" or "worker_identity_mismatch"||
    (hello&&(!reply.Ok||!reply.SupportsWorkerRegistry))) {
    if(WorkerIdentity!=null)Interlocked.Exchange(ref _workerIdentityFault,1);
    return false;
   }
   var expected=WorkerIdentity;
   // Only a successful discovery can bind a bridge. A concurrent hello
   // must agree with whichever incarnation bound the bridge first.
   if(expected==null&&hello)expected=Interlocked.CompareExchange(ref _workerIdentity,received,null)??received;
   if(expected!=received) {
    Interlocked.Exchange(ref _workerIdentityFault,1);
    return false;
   }
   return !HasWorkerIdentityFault;
  }
  async Task<GameplayBridgeResponse> ExchangeAsync(object request,bool eventReply) {
   var elapsed=Stopwatch.StartNew();
   GameplayBridgeResponse Failed(GameplayBridgeFailure failure)=>GameplayBridgeResponse.Failed(failure,elapsed.ElapsedMilliseconds);
   using var deadline=new CancellationTokenSource(TimeSpan.FromSeconds(2));
   try {
    var body=JsonSerializer.SerializeToUtf8Bytes(request); if(body.Length>65536) return Failed(GameplayBridgeFailure.InvalidRequest);
    using var sent=JsonDocument.Parse(body);var root=sent.RootElement;
    string id=root.GetProperty("request_id").GetString();
    var op=root.GetProperty("op").GetString();
    if(root.GetProperty("v").GetInt32()!=2 || (eventReply ? op is not ("event" or "release" or "progression_committed" or "refresh_progression" or "loadout_selection_settled" or "hello") : op=="event"))return Failed(GameplayBridgeFailure.InvalidRequest);
    bool hello=op=="hello";
    if(_expectedWorkerId!=null) {
     var identity=WorkerIdentity;
     if(HasWorkerIdentityFault||(identity==null&&!hello))return Failed(GameplayBridgeFailure.WorkerIdentityMismatch);
     if(root.TryGetProperty("worker_identity",out _))return Failed(GameplayBridgeFailure.InvalidRequest);
     if(identity!=null)body=AttachWorkerIdentity(root,identity);
     if(body.Length>65536)return Failed(GameplayBridgeFailure.InvalidRequest);
    }
    using var socket=new TcpClient(); await socket.ConnectAsync(_host,_port,deadline.Token); using var stream=socket.GetStream();
    await stream.WriteAsync(body,deadline.Token); await stream.WriteAsync(new byte[]{10},deadline.Token);
    using var line=new MemoryStream(); var one=new byte[1];
    while(true) { if(await stream.ReadAsync(one,deadline.Token)!=1) return Failed(GameplayBridgeFailure.Unavailable); if(one[0]==10) break; if(line.Length==GameplayBridgeResponseCodec.MaximumResponseBytes) return Failed(GameplayBridgeFailure.MalformedResponse); line.WriteByte(one[0]); }
    if(!GameplayBridgeResponseCodec.TryParse(line.ToArray(),id,eventReply,out var reply))return Failed(GameplayBridgeFailure.MalformedResponse);
    if(_expectedWorkerId!=null&&!VerifyWorkerReply(reply,hello))return Failed(GameplayBridgeFailure.WorkerIdentityMismatch);
    reply.ElapsedMilliseconds=elapsed.ElapsedMilliseconds;return reply;
   } catch(OperationCanceledException) { return Failed(GameplayBridgeFailure.Timeout); }
   catch(Exception e) when(e is IOException || e is SocketException) { return Failed(GameplayBridgeFailure.Unavailable); }
   catch(Exception e) when(e is JsonException || e is InvalidOperationException || e is FormatException) { return Failed(GameplayBridgeFailure.InvalidRequest); }
  }
 }
}
