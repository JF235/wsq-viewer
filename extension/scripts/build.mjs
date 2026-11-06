// Bundle the extension for the desktop (Node) and web (web worker) extension hosts.
//   node scripts/build.mjs [--production] [--tests]
import * as esbuild from 'esbuild';
import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ext = join(dirname(fileURLToPath(import.meta.url)), '..');
const production = process.argv.includes('--production');
const tests = process.argv.includes('--tests');

for (const f of ['wsq-scalar.wasm', 'wsq-simd.wasm']) {
  if (!existsSync(join(ext, 'dist', 'wasm', f))) {
    console.error(`dist/wasm/${f} is missing: run npm run build:wasm first`);
    process.exit(1);
  }
}

const common = {
  absWorkingDir: ext,
  bundle: true,
  external: ['vscode'],
  format: 'cjs',
  target: 'es2022',
  minify: production,
  sourcemap: production ? false : 'linked',
  logLevel: 'warning',
};

await esbuild.build({ ...common, entryPoints: ['src/extension.ts'], platform: 'node', outfile: 'dist/node/extension.js' });
await esbuild.build({ ...common, entryPoints: ['src/extension.ts'], platform: 'browser', outfile: 'dist/web/extension.js' });
if (tests) {
  await esbuild.build({ ...common, entryPoints: ['test/integration/suite.ts'], platform: 'browser', outfile: 'dist/test/suite.js' });
}
console.log(`built${production ? ' (production)' : ''}${tests ? ' with tests' : ''}`);
