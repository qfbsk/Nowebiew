// tools/bmp2png.js — 24bpp BMP → PNG（Node 内置 zlib，零依赖）
// 用法: node bmp2png.js in.bmp out.png
const fs = require("fs");
const zlib = require("zlib");

const [inp, outp] = process.argv.slice(2);
const buf = fs.readFileSync(inp);
if (buf[0] !== 0x42 || buf[1] !== 0x4d) { console.error("not BMP"); process.exit(1); }
const dataOff = buf.readUInt32LE(10);
const w = buf.readInt32LE(18), h = buf.readInt32LE(22);
const bpp = buf.readUInt16LE(28);
if (bpp !== 24) { console.error("need 24bpp, got " + bpp); process.exit(1); }
const row = Math.ceil(w * 3 / 4) * 4; /* 注意：JS 无整数除法，必须 Math.ceil/ floor，否则 675.75*4=2703 造成对角线剪切 */
// 自下而上存储 → 转自上而下 RGB
const raw = Buffer.alloc((w * 3 + 1) * h);
for (let y = 0; y < h; y++) {
  const srcY = h - 1 - y;
  raw[y * (w * 3 + 1)] = 0; // filter none
  for (let x = 0; x < w; x++) {
    const si = dataOff + srcY * row + x * 3;
    const di = y * (w * 3 + 1) + 1 + x * 3;
    raw[di] = buf[si + 2]; raw[di + 1] = buf[si + 1]; raw[di + 2] = buf[si];
  }
}
// CRC32
const CRC_T = [];
for (let n = 0; n < 256; n++) { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; CRC_T[n] = c >>> 0; }
function crc32(b) { let c = 0xffffffff; for (let i = 0; i < b.length; i++) c = CRC_T[(c ^ b[i]) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; }
function chunk(type, data) {
  const len = Buffer.alloc(4); len.writeUInt32LE(data.length, 0); // 下方再转大端
  len.writeUInt32BE(data.length, 0);
  const td = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(td), 0);
  return Buffer.concat([len, td, crc]);
}
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4);
ihdr[8] = 8; ihdr[9] = 2; ihdr[10] = 0; ihdr[11] = 0; ihdr[12] = 0;
const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", zlib.deflateSync(raw, { level: 6 })),
  chunk("IEND", Buffer.alloc(0)),
]);
fs.writeFileSync(outp, png);
console.log(`OK ${w}x${h} -> ${outp}`);
