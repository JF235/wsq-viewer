import assert from 'node:assert/strict';
import { test } from 'node:test';
import { inflateSync } from 'node:zlib';
import { crc32, encodePng } from '../../src/png.ts';
import { ridges } from './images.ts';

interface Chunk { type: string; data: Uint8Array }

function chunks(png: Uint8Array): Chunk[] {
  assert.deepEqual(png.subarray(0, 8), new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]));
  const view = new DataView(png.buffer, png.byteOffset, png.byteLength);
  const out: Chunk[] = [];
  for (let at = 8; at < png.length;) {
    const len = view.getUint32(at);
    const type = String.fromCharCode(...png.subarray(at + 4, at + 8));
    const crc = (crc32(png.subarray(at + 4, at + 8 + len)) ^ 0xffffffff) >>> 0;
    assert.equal(view.getUint32(at + 8 + len), crc, `CRC of ${type}`);
    out.push({ type, data: png.subarray(at + 8, at + 8 + len) });
    at += 12 + len;
  }
  return out;
}

function unfilter(raw: Uint8Array, width: number, height: number): Uint8Array {
  const px = new Uint8Array(width * height);
  for (let y = 0; y < height; y++) {
    const f = raw[y * (width + 1)];
    for (let x = 0; x < width; x++) {
      const a = x > 0 ? px[y * width + x - 1] : 0;
      const b = y > 0 ? px[(y - 1) * width + x] : 0;
      const c = x > 0 && y > 0 ? px[(y - 1) * width + x - 1] : 0;
      const p = a + b - c;
      const paeth = Math.abs(p - a) <= Math.abs(p - b) && Math.abs(p - a) <= Math.abs(p - c) ? a : Math.abs(p - b) <= Math.abs(p - c) ? b : c;
      const pred = [0, a, b, (a + b) >> 1, paeth][f];
      px[y * width + x] = (raw[y * (width + 1) + 1 + x] + pred) & 0xff;
    }
  }
  return px;
}

test('a grey image survives the PNG round trip, with every filter type', async () => {
  const [w, h] = [301, 203];
  const px = ridges(w, h);
  px.fill(7, 0, w); // a flat first row
  const png = await encodePng(px, w, h, 500);
  const cs = chunks(png);
  assert.deepEqual(cs.map(c => c.type), ['IHDR', 'pHYs', 'IDAT', 'IEND']);
  const ihdr = new DataView(cs[0].data.buffer, cs[0].data.byteOffset);
  assert.equal(ihdr.getUint32(0), w);
  assert.equal(ihdr.getUint32(4), h);
  assert.deepEqual([...cs[0].data.subarray(8)], [8, 0, 0, 0, 0]);
  const raw = new Uint8Array(inflateSync(cs[2].data));
  assert.equal(raw.length, (w + 1) * h);
  assert.deepEqual(unfilter(raw, w, h), px);
});

test('the resolution goes into pHYs in pixels per metre', async () => {
  const cs = chunks(await encodePng(new Uint8Array(4), 2, 2, 500));
  const phys = new DataView(cs[1].data.buffer, cs[1].data.byteOffset);
  assert.equal(phys.getUint32(0), 19685);
  assert.equal(phys.getUint32(4), 19685);
  assert.equal(cs[1].data[8], 1);
});

test('without a resolution there is no pHYs chunk', async () => {
  const cs = chunks(await encodePng(new Uint8Array(4), 2, 2));
  assert.deepEqual(cs.map(c => c.type), ['IHDR', 'IDAT', 'IEND']);
});

test('fingerprint-like images compress', async () => {
  const [w, h] = [512, 512];
  const png = await encodePng(ridges(w, h), w, h);
  assert.ok(png.length < w * h * 0.8, `${png.length} bytes`);
});
