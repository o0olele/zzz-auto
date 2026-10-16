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
if (release) {
  // 发布目录必须自带邮件资源；只在构建机读取参考仓和模型缓存。
  cargo(['run', '--locked', '-p', 'od-tools', '--', 'mail-bundle', '--reference',
    process.env.ZZZ_AUTO_REFERENCE ?? path.join(root, '../ZenlessZoneZero-OneDragon')]);
  cargo(['run', '--locked', '-p', 'od-tools', '--', 'map-bundle', '--reference',
    process.env.ZZZ_AUTO_REFERENCE ?? path.join(root, '../ZenlessZoneZero-OneDragon')]);
}
cargo(['build', '--locked', '-p', 'zzz-engine', ...(release ? ['--release'] : [])]);
if (release) {
  const check = spawnSync(path.join(root, 'target/release/zzz-engine.exe'),
    ['--root', root, 'mail-bundle-check'], { cwd: root, stdio: 'inherit', shell: false });
  if (check.error) throw check.error;
  if (check.status !== 0) process.exit(check.status ?? 1);
  const mapCheck = spawnSync(path.join(root, 'target/release/zzz-engine.exe'),
    ['--root', root, 'map-bundle-check', '--bundle', path.join(root, 'resources/generated/map-preview')], { cwd: root, stdio: 'inherit', shell: false });
  if (mapCheck.error) throw mapCheck.error;
  if (mapCheck.status !== 0) process.exit(mapCheck.status ?? 1);
}
const destination = path.join(root, 'apps/desktop/src-tauri/binaries');
mkdirSync(destination, { recursive: true });
copyFileSync(path.join(root, `target/${release ? 'release' : 'debug'}/zzz-engine.exe`), path.join(destination, 'zzz-auto-engine-x86_64-pc-windows-msvc.exe'));
