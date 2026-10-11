import sys, struct

def load_bmp(p):
    with open(p, 'rb') as f:
        d = f.read()
    off = struct.unpack('<I', d[10:14])[0]
    w = struct.unpack('<i', d[18:22])[0]
    h = struct.unpack('<i', d[22:26])[0]
    bpp = struct.unpack('<H', d[28:30])[0]
    assert bpp == 32, bpp
    row = ((w * 4 + 3) // 4) * 4
    topdown = h > 0
    h = abs(h)
    px = bytearray(w * h * 3)
    for y in range(h):
        sy = y if topdown else h - 1 - y
        ro = off + sy * row
        po = y * w * 3
        for x in range(w):
            b, g, r = d[ro+x*4], d[ro+x*4+1], d[ro+x*4+2]
            px[po+x*3:po+x*3+3] = bytes((r, g, b))
    return w, h, px

a = load_bmp(sys.argv[1]); b = load_bmp(sys.argv[2])
w = min(a[0], b[0]); h = min(a[1], b[1])
step = int(sys.argv[3]) if len(sys.argv) > 3 else 3
diff = same = 0
for y in range(0, h, step):
    for x in range(0, w, step):
        ia = (y*a[0]+x)*3; ib = (y*b[0]+x)*3
        if a[2][ia:ia+3] == b[2][ib:ib+3]: same += 1
        else: diff += 1
tot = same + diff
print(f"diff={diff} ({100.0*diff/tot:.1f}%) total={tot} size={a[0]}x{a[1]}")
