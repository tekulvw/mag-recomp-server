using RT.Models.MAG;

static class QueueWireChecks
{
    static void Check(bool value,string reason) {if(!value)throw new Exception(reason);}
    static uint Bits(byte[] wire,ref int at,int width)
    {uint value=0;for(int i=0;i<width;i++,at++)value=(value<<1)|((uint)(wire[at/8]>>(7-at%8))&1);return value;}
    public static void Run()
    {
        var counts=Enumerable.Range(0,12).Select(i=>0x81020304u+(uint)i).ToArray();
        var bytes=MagMessageCodec.SerializeForApplication(new MagRetailJoinGameUpdate {
            TransactionId=42,Success=true,QueuedType=-2,QueueSizes=counts},20231);
        Check(bytes.Length==120&&bytes[0]==0&&bytes[1]==0x14,"Retail waiting layout");
        int at=560;
        Check(Bits(bytes,ref at,1)==1&&Bits(bytes,ref at,8)==254,"Bool/enum width or order");
        foreach(var count in counts)Check(Bits(bytes,ref at,32)==count,"Unaligned matrix");
        Check(at==953&&Bits(bytes,ref at,7)==0,"Padding");
        Check(MagMessageCodec.TryParseForApplication(bytes,20231,out var parsed)&&parsed is MagRetailJoinGameUpdate update&&
            update.TransactionId==42&&update.QueuedType==-2&&update.QueueSizes.SequenceEqual(counts),"Typed waiting update");
        var bad=(byte[])bytes.Clone();bad[^1]|=1;
        Check(!MagMessageCodec.TryParseForApplication(bad,20231,out _),"Nonzero padding accepted");
        Check(!MagMessageCodec.TryParseForApplication(bytes[..^1],20231,out _),"Truncated waiting packet");
        Check(!MagMessageCodec.TryParseForApplication(bytes.Concat(new byte[1]).ToArray(),20231,out _),"Trailing byte accepted");
        bytes=MagMessageCodec.SerializeForApplication(new MagRetailCancelJoinRequest {TransactionId=99,PartyId=0x12345678},20231);
        Check(bytes.Length==74&&bytes[1]==0x15&&bytes[^4..].SequenceEqual(new byte[]{0x12,0x34,0x56,0x78}),"Cancel layout");
        Check(MagMessageCodec.TryParseClientForApplication(bytes,20231,out parsed)&&parsed is MagRetailCancelJoinRequest cancel&&cancel.PartyId==0x12345678,"Typed cancel");
        Check(!MagMessageCodec.TryParseClientForApplication(bytes,20230,out _),"Retail cancel accepted by prototype");
    }
}
