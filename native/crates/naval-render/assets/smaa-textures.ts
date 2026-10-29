// Writes SMAA's lookup textures for naval-render (`src/antialiasing.rs`) from the PNGs three r185 embeds in
// examples/jsm/tsl/display/SMAANode.js: the area texture (160 x 560, the RG of an RGB PNG) and the search texture
// (66 x 33, grey), as raw channels, zlib-compressed. From the repository root after `bun run bootstrap`:
//   bun native/crates/naval-render/assets/smaa-textures.ts . native/crates/naval-render/assets
import { readFileSync, writeFileSync } from 'node:fs';
import { inflateSync, deflateSync } from 'node:zlib';

const [root, outDir] = process.argv.slice(2);
const src = readFileSync(`${root}/node_modules/three/examples/jsm/tsl/display/SMAANode.js`, 'utf8');
const pngs = [...src.matchAll(/data:image\/png;base64,([A-Za-z0-9+\/=]+)/g)].map(m => Buffer.from(m[1], 'base64'));

/** An 8-bit, non-interlaced PNG's channels, filters undone. */
function decode(png: Buffer) {
  let o = 8, w = 0, h = 0, type = 0;
  const idat: Buffer[] = [];
  while (o < png.length) {
    const len = png.readUInt32BE(o), t = png.toString('latin1', o + 4, o + 8), body = png.subarray(o + 8, o + 8 + len);
    if (t === 'IHDR') {
      w = body.readUInt32BE(0); h = body.readUInt32BE(4); type = body[9];
      if (body[8] !== 8 || body[12] !== 0) throw new Error('unsupported PNG');
    } else if (t === 'IDAT') idat.push(body);
    o += 12 + len;
  }
  const channels = type === 2 ? 3 : type === 6 ? 4 : type === 0 ? 1 : type === 4 ? 2 : 0;
  const raw = inflateSync(Buffer.concat(idat)), stride = w * channels, out = new Uint8Array(h * stride);
  for (let y = 0; y < h; y++) {
    const filter = raw[y * (stride + 1)], line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x++) {
      const a = x >= channels ? out[y * stride + x - channels] : 0, b = y > 0 ? out[(y - 1) * stride + x] : 0;
      const c = x >= channels && y > 0 ? out[(y - 1) * stride + x - channels] : 0;
      let v = line[x];
      if (filter === 1) v += a;
      else if (filter === 2) v += b;
      else if (filter === 3) v += (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      out[y * stride + x] = v & 255;
    }
  }
  return { w, h, channels, data: out };
}

const area = decode(pngs[0]), search = decode(pngs[1]);
const rg = new Uint8Array(area.w * area.h * 2);
for (let i = 0; i < area.w * area.h; i++) { rg[i * 2] = area.data[i * 3]; rg[i * 2 + 1] = area.data[i * 3 + 1]; }
console.log(`area ${area.w}x${area.h}x${area.channels}, search ${search.w}x${search.h}x${search.channels}`);
writeFileSync(`${outDir}/smaa_area_160x560_rg8.zlib`, deflateSync(rg, { level: 9 }));
writeFileSync(`${outDir}/smaa_search_66x33_r8.zlib`, deflateSync(search.data, { level: 9 }));
