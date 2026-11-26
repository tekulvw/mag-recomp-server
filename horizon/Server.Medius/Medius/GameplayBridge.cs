using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
namespace Server.Medius {
 // One bounded JSONL request per connection. Never include credentials in diagnostics.
 public sealed class GameplayBridge {
  readonly string _host; readonly int _port;
  // Bridge deadlines. Ordinary control requests keep two seconds. An event may
  // wait out a short gameplay world-lock or storage stall instead of losing its
  // reply. World frames include a heartbeat every second; the read deadline only
  // detects a worker that stopped producing them, because a gap releases every
  // player. Live engine stalls of two to four seconds ended matches at 2/3 s.
  public static readonly TimeSpan RequestDeadline=TimeSpan.FromSeconds(2);
  public static readonly TimeSpan EventDeadline=TimeSpan.FromSeconds(6);
  public static readonly TimeSpan WorldFrameDeadline=TimeSpan.FromSeconds(12);
  // Each request opens a new connection. Resolving the worker's service name for
  // every one sent several requests per second through the cluster resolver,
  // and a lost lookup took the resolver's five-second retry. Keep the answer
  // and refresh it in the background once it is older than AddressLifetime:
  // the first request after an idle spell must not wait on the resolver inside
  // its two-second deadline. Resolve again, before giving up, when a connection
  // to an answer older than AddressLifetime fails, and forget any answer whose
  // connection fails.
  public static readonly TimeSpan AddressLifetime=TimeSpan.FromSeconds(30);
  // Bounds one lookup on its own, so a request that stops waiting leaves it to
  // finish and still fill the cache.
  public static readonly TimeSpan LookupDeadline=TimeSpan.FromSeconds(15);
  // Requests share a lookup younger than this. An older one is probably waiting
  // out the resolver's retry, so a new request starts another lookup beside it.
  public static readonly TimeSpan LookupShareWindow=TimeSpan.FromSeconds(1);
  sealed record ResolvedAddresses(IPAddress[] Addresses,long ExpiresAt) {
   public bool Expired=>Environment.TickCount64>=ExpiresAt;
  }
  Func<string,CancellationToken,Task<IPAddress[]>> _resolve=(host,token)=>Dns.GetHostAddressesAsync(host,token);
  ResolvedAddresses _addresses;
  readonly object _lookupGate=new();
  Task<ResolvedAddresses> _lookup;
  long _lookupStarted;
  readonly string _expectedWorkerId;
  GameplayWorkerIdentity _workerIdentity;
  int _workerIdentityFault;
  public GameplayWorkerIdentity WorkerIdentity => Volatile.Read(ref _workerIdentity);
  public bool HasWorkerIdentityFault => Volatile.Read(ref _workerIdentityFault)!=0;
  int _perRecipientReplies;
  /// <summary>Reply bounds for this worker: per-recipient once its latest successful hello advertised
  /// <see cref="GameplayReplyLimits.PerRecipientCapability"/>, otherwise the legacy shared bounds.</summary>
  public GameplayReplyLimits ReplyLimits => Volatile.Read(ref _perRecipientReplies)!=0?GameplayReplyLimits.PerRecipient:GameplayReplyLimits.Legacy;
  public GameplayBridge(string host,int port) { if(string.IsNullOrWhiteSpace(host)||port<1||port>65535) throw new ArgumentException("Invalid gameplay endpoint."); _host=host;_port=port; }
  public GameplayBridge(string host,int port,string expectedWorkerId) : this(host,port) {
   if(!GameplayWorkerIdentity.IsValidIdentifier(expectedWorkerId))throw new ArgumentException("Invalid gameplay worker ID.",nameof(expectedWorkerId));
   _expectedWorkerId=expectedWorkerId;
  }
  public Task<GameplayBridgeResponse> RequestControlAsync(object request) => ExchangeAsync(request,false);
  /// <summary>A control request with its own reply deadline, for example admission's configured one.</summary>
  public Task<GameplayBridgeResponse> RequestControlAsync(object request,TimeSpan deadline) {
   if(deadline<=TimeSpan.Zero)throw new ArgumentOutOfRangeException(nameof(deadline));
   return ExchangeAsync(request,false,deadline);
  }
  public async Task<bool> RequestAsync(object request) => (await ExchangeAsync(request,false))?.Ok == true;
  public Task<GameplayBridgeResponse> RequestHelloAsync(object request) => ExchangeAsync(request,true);
  public Task<GameplayBridgeResponse> RequestLoadoutSelectionSettlementAsync(object request) => ExchangeAsync(request,true);
  public Task<GameplayBridgeResponse> RequestEventAsync(object request) => ExchangeAsync(request,true,EventDeadline);
  public Task<GameplayBridgeResponse> RequestProgressionAsync(object request) => ExchangeAsync(request,true);
  public Task<GameplayBridgeResponse> RequestProgressionRefreshAsync(object request) => ExchangeAsync(request,true);
  public Task<GameplayBridgeResponse> RequestReleaseAsync(object request) => ExchangeAsync(request,true);
  // This connection owns only unsolicited world ticks. Ordinary requests retain
  // their existing request/reply path and are never replayed after a timeout.
  public Task SubscribeWorldAsync(uint worldId,string requestId,Func<GameplayBridgeResponse,Task> receive,CancellationToken cancellationToken)=>
   SubscribeWorldAsync(worldId,requestId,false,null,receive,cancellationToken);
  /// <summary>Subscribe to world frames. With <paramref name="positions"/> (world_stream_resume_v1)
  /// every frame carries its position; <paramref name="resumeAfter"/> asks the same worker process
  /// to replay the frames after that position first. A worker that cannot (another process, or
  /// frames no longer retained) refuses with <see cref="GameplayWorldResumeRefusedException"/>.</summary>
  public Task SubscribeWorldAsync(uint worldId,string requestId,bool positions,GameplayWorldStreamPosition resumeAfter,Func<GameplayBridgeResponse,Task> receive,CancellationToken cancellationToken)=>
   SubscribeWorldAsync(worldId,requestId,positions,resumeAfter,false,receive,cancellationToken);
  /// <summary>As above; with <paramref name="voiceGroups"/> (only toward a worker that advertised
  /// voice_groups_v1) its frames may also carry the world's voice relay table.</summary>
  public Task SubscribeWorldAsync(uint worldId,string requestId,bool positions,GameplayWorldStreamPosition resumeAfter,bool voiceGroups,Func<GameplayBridgeResponse,Task> receive,CancellationToken cancellationToken)=>
   SubscribeWorldAsync(worldId,requestId,new GameplayWorldSubscription(positions,resumeAfter,voiceGroups),receive,cancellationToken);
  /// <summary>Subscribe with every option. A refusal names the worker's reason in
  /// <see cref="GameplayWorldSubscriptionRefusedException"/>.</summary>
  public async Task SubscribeWorldAsync(uint worldId,string requestId,GameplayWorldSubscription options,Func<GameplayBridgeResponse,Task> receive,CancellationToken cancellationToken) {
   if(string.IsNullOrEmpty(requestId)||receive==null||options==null)throw new ArgumentException("Invalid world subscription.");
   bool positions=options.Positions;var resumeAfter=options.ResumeAfter;bool voiceGroups=options.VoiceGroups;
   if(options.ControlPlane!=null&&!GameplayWorldSubscription.IsValidControlPlane(options.ControlPlane))throw new ArgumentException("Invalid control plane.");
   var identity=WorkerIdentity;
   if(_expectedWorkerId!=null&&(identity==null||HasWorkerIdentityFault))throw new GameplayWorkerIdentityException();
   var request=new Dictionary<string,object>{["v"]=2,["request_id"]=requestId,["op"]="subscribe_world",["world_id"]=worldId};
   if(positions||resumeAfter!=null)request["positions"]=true;
   if(resumeAfter!=null)request["resume"]=new Dictionary<string,object>{["epoch"]=resumeAfter.Epoch,["after"]=resumeAfter.Sequence};
   if(voiceGroups)request["voice_groups"]=true;
   if(options.ControlPlane!=null)request["control_plane"]=options.ControlPlane;
   if(options.DrainNotices)request["drain_notices"]=true;
   var body=identity==null?JsonSerializer.SerializeToUtf8Bytes(request):AttachWorkerIdentity(JsonSerializer.SerializeToElement(request),identity);
   TcpClient connected;
   using(var connect=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken)) {
    connect.CancelAfter(RequestDeadline);connected=await ConnectAsync(connect.Token);
   }
   using var socket=connected;
   using var stream=socket.GetStream();
   await stream.WriteAsync(body,cancellationToken);await stream.WriteAsync(new byte[]{10},cancellationToken);
   // Keep framing bounds and a fresh deadline for each heartbeat/frame. A peer
   // that sends partial bytes forever cannot keep a stale world marked ready.
   var buffer=new byte[8192];int offset=0,available=0;
   // The subscription keeps the bounds of the hello that preceded it.
   var limits=ReplyLimits;
   while(true) {
    using var deadline=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);deadline.CancelAfter(WorldFrameDeadline);
    using var line=new MemoryStream();
    while(true) {
     if(offset==available) {
      available=await stream.ReadAsync(buffer,deadline.Token);offset=0;
      if(available==0)throw new IOException("Gameplay world stream closed.");
     }
     int end=Array.IndexOf(buffer,(byte)10,offset,available-offset);
     int count=(end<0?available:end)-offset;
     if(line.Length+count>limits.ResponseBytes)throw new InvalidDataException("Gameplay world stream frame too large.");
     line.Write(buffer,offset,count);offset+=count;
     if(end>=0){offset++;break;}
    }
    if(!GameplayBridgeResponseCodec.TryParse(line.ToArray(),requestId,true,limits,out var reply))throw new InvalidDataException("Invalid gameplay world stream response.");
    if(identity!=null&&!VerifyWorkerReply(reply,false))throw new GameplayWorkerIdentityException();
    if(!reply.Ok) {
     if(reply.Error=="world_resume_unavailable")throw new GameplayWorldResumeRefusedException();
     // The previous subscription has not been retired yet; try again shortly.
     if(reply.Error=="world_subscription_exists")throw new IOException("Gameplay world subscription is still held.");
     throw new GameplayWorldSubscriptionRefusedException(reply.Error);
    }
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
  /// <summary>Connect to a known address of the worker. A failed connection
  /// forgets the cached answer, so a moved Service or worker is found again. When
  /// that answer was older than <see cref="AddressLifetime"/>, resolve the name and
  /// connect once more inside the same deadline; nothing has been sent yet.</summary>
  async Task<TcpClient> ConnectAsync(CancellationToken token) {
   var entry=await AddressesAsync(token);
   var socket=new TcpClient();
   try { await socket.ConnectAsync(entry.Addresses,_port,token); return socket; }
   catch(Exception error) when(error is SocketException||error is OperationCanceledException) {
    socket.Dispose();
    Interlocked.CompareExchange(ref _addresses,null,entry);
    if(token.IsCancellationRequested||!entry.Expired)throw;
   }
   entry=await LookupAsync().WaitAsync(token);
   socket=new TcpClient();
   try { await socket.ConnectAsync(entry.Addresses,_port,token); return socket; }
   catch(Exception error) when(error is SocketException||error is OperationCanceledException) {
    socket.Dispose();
    Interlocked.CompareExchange(ref _addresses,null,entry);
    throw;
   }
  }
  async Task<ResolvedAddresses> AddressesAsync(CancellationToken token) {
   if(IPAddress.TryParse(_host,out var literal))return new ResolvedAddresses(new[]{literal},long.MaxValue);
   var cached=Volatile.Read(ref _addresses);
   if(cached==null)return await LookupAsync().WaitAsync(token);
   if(cached.Expired)_=RefreshAddressesAsync();
   return cached;
  }
  async Task RefreshAddressesAsync() {
   // A failed refresh keeps the last answer; a failed connection forgets it.
   // Nothing awaits this refresh, so it observes every failure itself.
   try { await LookupAsync(false); }
   catch(Exception) {}
  }
  /// <summary>Requests close together share one lookup. A lookup outlives a request
  /// that stops waiting for it, so its answer still reaches the cache.
  /// A background refresh never starts a second lookup beside one in flight.</summary>
  Task<ResolvedAddresses> LookupAsync(bool waiting=true) {
   lock(_lookupGate) {
    long now=Environment.TickCount64;
    if(_lookup==null||_lookup.IsCompleted||(waiting&&now-_lookupStarted>=(long)LookupShareWindow.TotalMilliseconds)) {
     _lookup=ResolveAsync();_lookupStarted=now;
     // Waiters may all have given up; a failure must still count as observed.
     _=_lookup.ContinueWith(lookup=>_=lookup.Exception,TaskContinuationOptions.OnlyOnFaulted|TaskContinuationOptions.ExecuteSynchronously);
    }
    return _lookup;
   }
  }
  async Task<ResolvedAddresses> ResolveAsync() {
   await Task.Yield();
   // The deadline bounds the wait even where the platform resolver cannot be cancelled.
   using var deadline=new CancellationTokenSource(LookupDeadline);
   var resolved=await _resolve(_host,deadline.Token).WaitAsync(deadline.Token);
   if(resolved==null||resolved.Length==0)throw new SocketException((int)SocketError.HostNotFound);
   var entry=new ResolvedAddresses(resolved,Environment.TickCount64+(long)AddressLifetime.TotalMilliseconds);
   Volatile.Write(ref _addresses,entry);
   return entry;
  }
  Task<GameplayBridgeResponse> ExchangeAsync(object request,bool eventReply)=>ExchangeAsync(request,eventReply,RequestDeadline);
  async Task<GameplayBridgeResponse> ExchangeAsync(object request,bool eventReply,TimeSpan timeout) {
   var elapsed=Stopwatch.StartNew();
   bool sending=false;
   GameplayBridgeResponse Failed(GameplayBridgeFailure failure)=>MarkUnsent(GameplayBridgeResponse.Failed(failure,elapsed.ElapsedMilliseconds));
   GameplayBridgeResponse MarkUnsent(GameplayBridgeResponse reply) { reply.RequestNotSent=!sending; return reply; }
   using var deadline=new CancellationTokenSource(timeout);
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
    using var socket=await ConnectAsync(deadline.Token); using var stream=socket.GetStream();
    sending=true;
    await stream.WriteAsync(body,deadline.Token); await stream.WriteAsync(new byte[]{10},deadline.Token);
    using var line=new MemoryStream(); var one=new byte[1];
    var limits=ReplyLimits;
    while(true) { if(await stream.ReadAsync(one,deadline.Token)!=1) return Failed(GameplayBridgeFailure.Unavailable); if(one[0]==10) break; if(line.Length==limits.ResponseBytes) return Failed(GameplayBridgeFailure.MalformedResponse); line.WriteByte(one[0]); }
    if(!GameplayBridgeResponseCodec.TryParse(line.ToArray(),id,eventReply,limits,out var reply))return Failed(GameplayBridgeFailure.MalformedResponse);
    if(_expectedWorkerId!=null&&!VerifyWorkerReply(reply,hello))return Failed(GameplayBridgeFailure.WorkerIdentityMismatch);
    // A restarted worker answers a new hello; its capabilities set the bounds from then on.
    if(hello&&reply.Ok)Volatile.Write(ref _perRecipientReplies,reply.SupportsPerRecipientReplyBudget?1:0);
    reply.ElapsedMilliseconds=elapsed.ElapsedMilliseconds;return reply;
   } catch(OperationCanceledException) { return Failed(GameplayBridgeFailure.Timeout); }
   catch(Exception e) when(e is IOException || e is SocketException) { return Failed(GameplayBridgeFailure.Unavailable); }
   catch(Exception e) when(e is JsonException || e is InvalidOperationException || e is FormatException) { return Failed(GameplayBridgeFailure.InvalidRequest); }
  }
 }
 /// <summary>Options of one subscribe_world. <see cref="ControlPlane"/> names the subscribing Horizon
 /// process (only toward a worker that advertised control_plane_rebind_v1): a worker that last served
 /// another one releases what that process admitted. <see cref="DrainNotices"/> asks for
 /// worker_draining on frames (worker_drain_v1).</summary>
 public sealed record GameplayWorldSubscription(bool Positions=false,GameplayWorldStreamPosition ResumeAfter=null,bool VoiceGroups=false,
  string ControlPlane=null,bool DrainNotices=false) {
  public static bool IsValidControlPlane(string value)=>value!=null&&value.Length>=16&&value.Length<=64&&
   value.All(c=>char.IsAsciiLetterOrDigit(c)||c=='-'||c=='_');
 }
 /// <summary>The worker answered subscribe_world with an error, for example
 /// subscription_world_mismatch: it still holds players of another world ID. An
 /// <see cref="InvalidDataException"/>, as before, with the worker's reason kept.</summary>
 public sealed class GameplayWorldSubscriptionRefusedException : InvalidDataException {
  public string Reason { get; }
  public GameplayWorldSubscriptionRefusedException(string reason) : base($"Gameplay worker refused the world subscription: {Describe(reason)}.") { Reason=Describe(reason); }
  // The reason comes from the worker; keep only a short identifier in logs.
  static string Describe(string reason)=>reason!=null&&reason.Length<=64&&reason.All(c=>char.IsAsciiLetterOrDigit(c)||c=='_'||c=='-')?reason:"unspecified";
 }
 /// <summary>The worker cannot replay the requested frames: it is another process, or
 /// the frames are no longer retained. Sessions that depended on them must end.</summary>
 public sealed class GameplayWorldResumeRefusedException : IOException {
  public GameplayWorldResumeRefusedException() : base("Gameplay world stream cannot resume.") {}
 }
}
