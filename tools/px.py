import struct, sys
d = open(sys.argv[1], 'rb').read()
off = struct.unpack_from('<I', d, 10)[0]
w = struct.unpack_from('<i', d, 18)[0]
h = struct.unpack_from('<i', d, 22)[0]
row = ((w * 3 + 3) // 4) * 4

def px(x, y):
    i = off + (h - 1 - y) * row + x * 3
    return (d[i + 2], d[i + 1], d[i])

print('size', w, h)
print('bg   =', px(450, 80))
print('badge=', px(88, 371))
print('quote=', px(450, 455))
print('btn  =', px(846, 27))
print('nav  =', px(35, 604))
