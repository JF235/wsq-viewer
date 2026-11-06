import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { WsqCodec, WsqError } from '../../src/codec.ts';
import { ridges } from './images.ts';

const wasm = (name: string) => new Uint8Array(readFileSync(new URL(`../../dist/wasm/wsq-${name}.wasm`, import.meta.url)));
const scalar = await WsqCodec.load(wasm('scalar'));
const simd = await WsqCodec.load(wasm('scalar'), wasm('simd'));

test('the SIMD module is picked when the engine supports it', () => {
  assert.equal(scalar.lanes, 1);
  assert.equal(simd.lanes, 4);
});

test('encode, info and decode round trip', () => {
  const [w, h] = [417, 289];
  const px = ridges(w, h);
  const stream = simd.encode(px, w, h, { bitrate: 0.75, ppi: 500 });
  assert.deepEqual(stream.subarray(0, 2), new Uint8Array([0xff, 0xa0]));

  const info = simd.info(stream);
  assert.equal(info.width, w);
  assert.equal(info.height, h);
  assert.equal(info.ppi, 500);
  assert.equal(info.bitrate, 0.75);
  assert.deepEqual(info.filters, [9, 7]);
  assert.equal(info.encoder, 2);
  assert.ok(info.comments.some(c => c.startsWith('NIST_COM')));

  const img = simd.decode(stream);
  assert.equal(img.width, w);
  assert.equal(img.height, h);
  assert.equal(img.ppi, 500);
  let se = 0;
  for (let i = 0; i < px.length; i++) {
    se += (img.pixels[i] - px[i]) ** 2;
  }
  assert.ok(se / px.length < 40, `mean squared error ${se / px.length}`);
});

test('scalar and SIMD modules agree bit for bit in exact mode', () => {
  const [w, h] = [640, 480];
  const px = ridges(w, h);
  const a = scalar.encode(px, w, h);
  const b = simd.encode(px, w, h);
  assert.deepEqual(a, b);
  assert.deepEqual(scalar.decode(a).pixels, simd.decode(a).pixels);
});

test('a resolution-less stream has no ppi', () => {
  const px = ridges(64, 64);
  const img = simd.decode(simd.encode(px, 64, 64));
  assert.equal(img.ppi, undefined);
});

test('invalid input raises WsqError with a message', () => {
  assert.throws(() => simd.decode(new Uint8Array([1, 2, 3])), (e: unknown) => e instanceof WsqError && e.message.length > 0);
  assert.throws(() => simd.info(new Uint8Array(0)), WsqError);
  const stream = simd.encode(ridges(100, 80), 100, 80);
  assert.throws(() => simd.decode(stream.subarray(0, stream.length / 2)), WsqError);
  assert.throws(() => simd.encode(new Uint8Array(10), 4, 4), WsqError);
});

test('the module keeps working after many calls (no leak of input buffers)', () => {
  const px = ridges(256, 256);
  const stream = simd.encode(px, 256, 256);
  const before = (simd as unknown as { w: { memory: WebAssembly.Memory } }).w.memory.buffer.byteLength;
  for (let i = 0; i < 200; i++) {
    simd.decode(stream);
  }
  const after = (simd as unknown as { w: { memory: WebAssembly.Memory } }).w.memory.buffer.byteLength;
  assert.equal(after, before);
});
