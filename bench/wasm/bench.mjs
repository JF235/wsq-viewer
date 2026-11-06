// Time the WebAssembly build of the codec in Node.
//   node bench.mjs <module.wasm> <reps> <file.wsq> [reference.raw] [--fast]
import { readFileSync } from 'node:fs';

const [mod, repsArg, file, ref, ...rest] = process.argv.slice(2);
const fast = rest.includes('--fast') || ref === '--fast' ? 1 : 0;
const { instance } = await WebAssembly.instantiate(readFileSync(mod), {});
const w = instance.exports;
const data = readFileSync(file);
const ptr = w.wsq_alloc(data.length);
new Uint8Array(w.memory.buffer, ptr, data.length).set(data);

let dims = w.wsq_decode(ptr, data.length, fast); // warm-up
if (!dims) throw new Error('decode failed');
const width = dims >>> 16, height = dims & 0xffff;
if (ref && ref !== '--fast') {
  const expect = readFileSync(ref);
  const got = new Uint8Array(w.memory.buffer, w.wsq_pixels(), width * height);
  let diff = 0;
  for (let i = 0; i < got.length; i++) if (got[i] !== expect[i]) diff++;
  console.log(`pixels differing from the reference: ${diff}`);
}
const reps = Number(repsArg);
const t = [];
for (let i = 0; i < reps; i++) {
  const t0 = process.hrtime.bigint();
  w.wsq_decode(ptr, data.length, fast);
  t.push(Number(process.hrtime.bigint() - t0) / 1e6);
}
t.sort((a, b) => a - b);
console.log(`${file.split('/').pop()} ${width}x${height} lanes=${w.wsq_backend()} best=${t[0].toFixed(3)} ms median=${t[reps >> 1].toFixed(3)} ms`);
