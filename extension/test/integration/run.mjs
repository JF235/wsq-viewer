// Run the integration suite in VS Code for the web, in headless Chromium (no display needed).
//   node test/integration/run.mjs        (after: node scripts/build.mjs --tests)
// WSQ_TEST_DATA_DIR moves the downloaded VS Code build and the fixtures elsewhere
// (default: .vscode-test-web inside the extension folder).
import { runTests } from '@vscode/test-web';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { WsqCodec } from '../../src/codec.ts';
import { encodePng } from '../../src/png.ts';
import { ridges } from '../unit/images.ts';

const ext = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const data = process.env.WSQ_TEST_DATA_DIR ?? join(ext, '.vscode-test-web');
const fixtures = join(data, 'workspace');

// Synthetic fixtures only: no real fingerprint is needed to test the editor.
rmSync(fixtures, { recursive: true, force: true });
mkdirSync(fixtures, { recursive: true });
const wasm = name => new Uint8Array(readFileSync(join(ext, 'dist', 'wasm', `wsq-${name}.wasm`)));
const codec = await WsqCodec.load(wasm('scalar'), wasm('simd'));
const image = (w, h, ppi) => codec.encode(ridges(w, h), w, h, { bitrate: 0.75, ppi });
const ridgesWsq = image(417, 289, 500);
writeFileSync(join(fixtures, 'ridges.wsq'), ridgesWsq);
writeFileSync(join(fixtures, 'other.wsq'), image(300, 200, 500));
writeFileSync(join(fixtures, 'UPPER.WSQ'), image(64, 48));
writeFileSync(join(fixtures, 'watched.wsq'), image(100, 80, 500));
writeFileSync(join(fixtures, 'broken.wsq'), ridgesWsq.subarray(0, ridgesWsq.length >> 1));
writeFileSync(join(fixtures, 'a.png'), await encodePng(ridges(120, 90), 120, 90));
writeFileSync(join(fixtures, 'b.png'), await encodePng(ridges(120, 90).reverse(), 120, 90));

await runTests({
  browserType: 'chromium',
  headless: true,
  quality: 'stable',
  extensionDevelopmentPath: ext,
  extensionTestsPath: join(ext, 'dist', 'test', 'suite.js'),
  folderPath: fixtures,
  testRunnerDataDir: join(data, 'runner'),
  printServerLog: false,
});
