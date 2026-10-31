"""Read-only bounded PSARC1.3 zlib member reader.
Adapted from mag_decomp/tools/psarc.py, with bounded zlib output and limits.
"""
import struct,zlib
from packfile import CollisionFormatError

MAX_ARCHIVE=2*1024**3
MAX_MEMBER=512*1024**2
class Psarc:
    def __init__(self,data):
        if len(data)<32 or len(data)>MAX_ARCHIVE:raise CollisionFormatError('archive size outside limit')
        self.data=data
        magic,version,compression,toc,size,count,block,flags=struct.unpack_from('>4sI4sIIIII',data)
        if (magic,version,compression,size)!=(b'PSAR',0x10003,b'zlib',30):raise CollisionFormatError('unsupported archive format')
        if block<2 or block>1024**2 or block&(block-1) or not 1<=count<=100_000:raise CollisionFormatError('archive count/block limit')
        width=((block-1).bit_length()+7)//8;start=32+size*count
        if not start<=toc<=len(data) or (toc-start)%width:raise CollisionFormatError('invalid archive table bounds')
        self.block=block;self.sizes=[int.from_bytes(data[p:p+width],'big') or block for p in range(start,toc,width)]
        self.entries=[]
        for i in range(count):
            at=32+i*size;entry=data[at:at+size]
            index=int.from_bytes(entry[16:20],'big');length=int.from_bytes(entry[20:25],'big');offset=int.from_bytes(entry[25:30],'big');blocks=(length+block-1)//block
            if length>MAX_MEMBER or index+blocks>len(self.sizes) or offset<toc or offset+sum(self.sizes[index:index+blocks])>len(data):raise CollisionFormatError('invalid archive member bounds')
            self.entries.append((index,length,offset,blocks))
        if self.entries[0][1]>10*1024**2:raise CollisionFormatError('archive name table too large')
        try:self.names=self.extract(0).decode('utf-8').splitlines()
        except UnicodeError as e:raise CollisionFormatError('invalid archive names') from e
        if len(self.names)!=count-1 or len(set(self.names))!=len(self.names):raise CollisionFormatError('invalid archive name count/duplicates')
    def extract(self,index):
        if not 0<=index<len(self.entries):raise CollisionFormatError('member index out of bounds')
        first,length,offset,blocks=self.entries[index];out=bytearray()
        for i in range(blocks):
            size=self.sizes[first+i];raw=self.data[offset:offset+size];expected=min(self.block,length-i*self.block)
            if size==expected:decoded=raw
            else:
                try:
                    decoder=zlib.decompressobj();decoded=decoder.decompress(raw,expected+1)
                    if len(decoded)!=expected or not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:raise CollisionFormatError('invalid decoded archive block')
                except zlib.error as e:raise CollisionFormatError('invalid compressed archive block') from e
            if len(decoded)!=expected:raise CollisionFormatError('archive block length mismatch')
            out.extend(decoded);offset+=size
        return bytes(out)
