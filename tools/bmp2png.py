import struct, sys, zlib

def bmp_to_png(src, dst):
    d = open(src, 'rb').read()
    off = struct.unpack('<I', d[10:14])[0]
    w = struct.unpack('<i', d[18:22])[0]
    h = struct.unpack('<i', d[22:26])[0]
    bpp = struct.unpack('<H', d[28:30])[0]
    row = ((w * (bpp // 8) + 3) // 4) * 4
    topdown = h < 0  # 正 biHeight=自底向上；负=自顶向下
    h = abs(h)
    raw = bytearray()
    for y in range(h):
        sy = y if topdown else h - 1 - y
        ro = off + sy * row
        raw.append(0)  # filter none
        for x in range(w):
            i = ro + x * (bpp // 8)
            raw += bytes((d[i + 2], d[i + 1], d[i]))
    def chunk(t, data):
        c = struct.pack('>I', len(data)) + t + data
        c += struct.pack('>I', zlib.crc32(t + data) & 0xffffffff)
        return c
    png = b'\x89PNG\r\n\x1a\n'
    png += chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0))
    png += chunk(b'IDAT', zlib.compress(bytes(raw), 6))
    png += chunk(b'IEND', b'')
    open(dst, 'wb').write(png)

if __name__ == '__main__':
    bmp_to_png(sys.argv[1], sys.argv[2])
    print('ok', sys.argv[2])
