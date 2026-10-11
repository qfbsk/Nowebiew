import struct, zlib, sys

def load_png(path):
    f = open(path, 'rb').read()
    pos = 8; w = h = 0; idat = b''
    while pos < len(f):
        ln = struct.unpack('>I', f[pos:pos+4])[0]
        typ = f[pos+4:pos+8]
        data = f[pos+8:pos+8+ln]
        if typ == b'IHDR':
            w, h = struct.unpack('>II', data[:8])
        elif typ == b'IDAT':
            idat += data
        pos += 12 + ln
    d = zlib.decompressobj()
    raw = d.decompress(idat) + d.flush()
    return w, h, raw

def main(path, samples):
    w, h, raw = load_png(path)
    print('size %dx%d rawlen=%d' % (w, h, len(raw)))
    stride = w * 3 + 1
    def px(x, y):
        o = y * stride + 1 + x * 3
        return (raw[o+2], raw[o+1], raw[o])
    # 逐行采样
    for y in samples:
        print('y=%d:' % y, ' '.join('%d,%d,%d' % px(x, y) for x in samples[y]))

if __name__ == '__main__':
    path = sys.argv[1]
    ys = [int(v) for v in sys.argv[2].split(',')]
    xs = [int(v) for v in sys.argv[3].split(',')]
    samples = {y: xs for y in ys}
    main(path, samples)
