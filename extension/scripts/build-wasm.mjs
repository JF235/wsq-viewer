// Build the codec as WebAssembly, with and without SIMD, into dist/wasm/.
// Needs Rust with the wasm32-unknown-unknown target (rustup target add wasm32-unknown-unknown).
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ext = join(dirname(fileURLToPath(import.meta.url)), '..');
const repo = join(ext, '..');
mkdirSync(join(ext, 'dist', 'wasm'), { recursive: true });

for (const [name, flags] of [['scalar', ''], ['simd', '-C target-feature=+simd128']]) {
  const targetDir = join(repo, 'target', `wasm-${name}`);
  execFileSync('cargo', ['build', '--release', '-p', 'wsq-wasm', '--target', 'wasm32-unknown-unknown', '--target-dir', targetDir], {
    cwd: repo,
    stdio: 'inherit',
    env: { ...process.env, RUSTFLAGS: flags, CARGO_PROFILE_RELEASE_DEBUG: 'false', CARGO_PROFILE_RELEASE_STRIP: 'symbols' },
  });
  const out = join(ext, 'dist', 'wasm', `wsq-${name}.wasm`);
  copyFileSync(join(targetDir, 'wasm32-unknown-unknown', 'release', 'wsq_wasm.wasm'), out);
  console.log(`${out}: ${(statSync(out).size / 1024).toFixed(0)} KB`);
}
