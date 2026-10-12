import { spawnSync } from 'node:child_process';
import { mkdirSync, copyFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));
const release = process.argv.includes('--release');
const cargo = (args) => {
  const result = spawnSync('cargo', args, { cwd: root, stdio: 'inherit', shell: false });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
};
cargo(['run', '--locked', '-p', 'od-tools', '--', 'types']);
if (process.argv.includes('--types-only')) process.exit(0);
cargo(['build', '--locked', '-p', 'zzz-engine', ...(release ? ['--release'] : [])]);
const destination = path.join(root, 'apps/desktop/src-tauri/binaries');
mkdirSync(destination, { recursive: true });
copyFileSync(path.join(root, `target/${release ? 'release' : 'debug'}/zzz-engine.exe`), path.join(destination, 'zzz-auto-engine-x86_64-pc-windows-msvc.exe'));
