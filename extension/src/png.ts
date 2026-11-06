// A small PNG writer for 8-bit grey images. Each row gets the filter that minimizes the sum of
// absolute differences (the heuristic of the PNG specification); the resolution, when known, goes
// into a pHYs chunk. Compression uses CompressionStream, available in Node 18+ and web workers.

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    t[n] = c >>> 0;
  }
  return t;
})();

export function crc32(bytes: Uint8Array, crc = 0xffffffff): number {
  for (let i = 0; i < bytes.length; i++) {
    crc = CRC_TABLE[(crc ^ bytes[i]) & 0xff] ^ (crc >>> 8);
  }
  return crc;
}

function chunk(type: string, data: Uint8Array): Uint8Array {
  const out = new Uint8Array(12 + data.length);
  const view = new DataView(out.buffer);
  view.setUint32(0, data.length);
  for (let i = 0; i < 4; i++) {
    out[4 + i] = type.charCodeAt(i);
  }
  out.set(data, 8);
  view.setUint32(8 + data.length, (crc32(out.subarray(4, 8 + data.length)) ^ 0xffffffff) >>> 0);
  return out;
}

function paeth(a: number, b: number, c: number): number {
  const p = a + b - c;
  const pa = Math.abs(p - a);
  const pb = Math.abs(p - b);
  const pc = Math.abs(p - c);
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
}

/** Filter the rows of a grey image: one filter-type byte followed by the filtered row. */
export function filterRows(pixels: Uint8Array, width: number, height: number): Uint8Array {
  const out = new Uint8Array((width + 1) * height);
  const zero = new Uint8Array(width);
  const candidates = Array.from({ length: 5 }, () => new Uint8Array(width));
  for (let y = 0; y < height; y++) {
    const row = pixels.subarray(y * width, (y + 1) * width);
    const up = y > 0 ? pixels.subarray((y - 1) * width, y * width) : zero;
    let best = 0;
    let bestCost = Infinity;
    for (let f = 0; f < 5; f++) {
      const c = candidates[f];
      let cost = 0;
      for (let x = 0; x < width; x++) {
        const a = x > 0 ? row[x - 1] : 0;
        const b = up[x];
        const d = x > 0 ? up[x - 1] : 0;
        const pred = f === 0 ? 0 : f === 1 ? a : f === 2 ? b : f === 3 ? (a + b) >> 1 : paeth(a, b, d);
        const v = (row[x] - pred) & 0xff;
        c[x] = v;
        cost += v < 128 ? v : 256 - v;
      }
      if (cost < bestCost) {
        bestCost = cost;
        best = f;
      }
    }
    out[y * (width + 1)] = best;
    out.set(candidates[best], y * (width + 1) + 1);
  }
  return out;
}

async function deflate(data: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([data as Uint8Array<ArrayBuffer>]).stream().pipeThrough(new CompressionStream('deflate'));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

/** Encode `width * height` grey pixels as a PNG file. */
export async function encodePng(pixels: Uint8Array, width: number, height: number, ppi?: number): Promise<Uint8Array> {
  const header = new Uint8Array(13);
  const hv = new DataView(header.buffer);
  hv.setUint32(0, width);
  hv.setUint32(4, height);
  header[8] = 8; // bit depth
  header[9] = 0; // colour type: grey
  const parts = [new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', header)];
  if (ppi && ppi > 0) {
    const phys = new Uint8Array(9);
    const pv = new DataView(phys.buffer);
    const perMetre = Math.round(ppi / 0.0254);
    pv.setUint32(0, perMetre);
    pv.setUint32(4, perMetre);
    phys[8] = 1; // unit: metre
    parts.push(chunk('pHYs', phys));
  }
  parts.push(chunk('IDAT', await deflate(filterRows(pixels, width, height))), chunk('IEND', new Uint8Array(0)));
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}
