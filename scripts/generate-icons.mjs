// Run with Node after npm ci: node scripts/generate-icons.mjs
// The checked-in SVG is the only artwork source; Tauri's pinned CLI rasterizes it.
import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join, dirname, basename } from 'node:path';
import { fileURLToPath } from 'node:url';
import { inflateSync } from 'node:zlib';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256];
const output = join(root, 'src-tauri/icons');
const scratch = mkdtempSync(join(tmpdir(), 'reelink-icons-'));
// Decode only the non-interlaced 8-bit RGB/RGBA PNGs from our pinned rasterizer.
function dibFrame(png, size) {
  if (!png.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) throw new Error('Invalid PNG');
  let channels;
  const chunks = [];
  for (let offset = 8; offset < png.length;) {
    const length = png.readUInt32BE(offset);
    if (offset + 12 + length > png.length) throw new Error('Truncated PNG');
    const type = png.toString('ascii', offset + 4, offset + 8);
    const data = png.subarray(offset + 8, offset + 8 + length);
    if (type === 'IHDR') {
      if (data.readUInt32BE(0) !== size || data.readUInt32BE(4) !== size || data[8] !== 8 ||
          ![2, 6].includes(data[9]) || data[10] || data[11] || data[12]) throw new Error('Unsupported PNG');
      channels = data[9] === 6 ? 4 : 3;
    }
    if (type === 'IDAT') chunks.push(data);
    offset += length + 12;
  }
  if (!channels || !chunks.length) throw new Error('Incomplete PNG');
  const raw = inflateSync(Buffer.concat(chunks));
  const stride = size * channels;
  if (raw.length !== size * (stride + 1)) throw new Error('Unexpected PNG data length');
  const pixels = Buffer.alloc(size * stride);
  const paeth = (a, b, c) => {
    const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
    return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
  };
  for (let y = 0; y < size; y++) {
    const filter = raw[y * (stride + 1)];
    if (filter > 4) throw new Error('Unsupported PNG filter');
    for (let x = 0; x < stride; x++) {
      const i = y * stride + x;
      const a = x >= channels ? pixels[i - channels] : 0;
      const b = y ? pixels[i - stride] : 0;
      const c = y && x >= channels ? pixels[i - stride - channels] : 0;
      pixels[i] = (raw[y * (stride + 1) + x + 1] + [0, a, b, Math.floor((a + b) / 2), paeth(a, b, c)][filter]) & 255;
    }
  }
  const maskStride = Math.ceil(size / 32) * 4;
  const bitmap = Buffer.alloc(40 + size * size * 4 + maskStride * size);
  bitmap.writeUInt32LE(40, 0);
  bitmap.writeInt32LE(size, 4);
  bitmap.writeInt32LE(size * 2, 8);
  bitmap.writeUInt16LE(1, 12);
  bitmap.writeUInt16LE(32, 14);
  bitmap.writeUInt32LE(size * size * 4 + maskStride * size, 20);
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
    const source = (y * size + x) * channels;
    const row = size - 1 - y;
    const target = 40 + (row * size + x) * 4;
    const alpha = channels === 4 ? pixels[source + 3] : 255;
    bitmap[target] = pixels[source + 2];
    bitmap[target + 1] = pixels[source + 1];
    bitmap[target + 2] = pixels[source];
    bitmap[target + 3] = alpha;
    if (alpha === 0) bitmap[40 + size * size * 4 + row * maskStride + Math.floor(x / 8)] |= 128 >> (x % 8);
  }
  return bitmap;
}
try {
  const artwork = readFileSync(join(root, 'public/reelink-logo.svg'), 'utf8');
  const images = sizes.map(size => {
    const master = artwork.match(new RegExp(`<symbol id="mark-${size}"[^>]*>([\\s\\S]*?)</symbol>`));
    if (!master) throw new Error(`Missing ${size}px optical master`);
    const svg = join(scratch, `master-${size}.svg`);
    writeFileSync(svg, `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}">${master[1]}</svg>`);
    const rasterOutput = join(scratch, String(size));
    const args = [join(root, 'node_modules/@tauri-apps/cli/tauri.js'), 'icon', svg, '--output', rasterOutput, '--png', String(size)];
    const result = spawnSync(process.execPath, args, { cwd: root, stdio: 'pipe' });
    if (result.status !== 0) throw new Error(`Tauri ${size}px rasterization failed: ${result.stderr}`);
    return readFileSync(join(rasterOutput, `${size}x${size}.png`));
  });
  const frames = images.map((png, i) => sizes[i] <= 64 ? dibFrame(png, sizes[i]) : png);
  const header = Buffer.alloc(6 + 16 * sizes.length);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(sizes.length, 4);
  let offset = header.length;
  sizes.forEach((size, index) => {
    const entry = 6 + 16 * index;
    header[entry] = header[entry + 1] = size === 256 ? 0 : size;
    header.writeUInt16LE(1, entry + 4);
    header.writeUInt16LE(32, entry + 6);
    header.writeUInt32LE(frames[index].length, entry + 8);
    header.writeUInt32LE(offset, entry + 12);
    offset += frames[index].length;
  });
  writeFileSync(join(output, 'icon.ico'), Buffer.concat([header, ...frames]));
  sizes.forEach((size, i) => writeFileSync(join(output, `${size}x${size}.png`), images[i]));
  writeFileSync(join(output, '128x128@2x.png'), images[sizes.indexOf(256)]);
  console.log('Generated pixel-aligned optical masters and Windows ICO (16–64px DIB; 128/256px PNG).');
} finally {
  if (dirname(resolve(scratch)) !== resolve(tmpdir()) || !basename(scratch).startsWith('reelink-icons-')) {
    throw new Error('Refusing to remove an unexpected temporary directory');
  }
  rmSync(scratch, { recursive: true, force: true });
}
