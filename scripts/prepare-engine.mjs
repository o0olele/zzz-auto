import { spawnSync } from 'node:child_process';
import { mkdirSync, copyFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));
const release = process.argv.includes('--release');
const targetDir = process.env.CARGO_TARGET_DIR ? path.resolve(root, process.env.CARGO_TARGET_DIR) : path.join(root, 'target');
const cargo = (args) => {
  const result = spawnSync('cargo', [args[0], '--target-dir', targetDir, ...args.slice(1)], { cwd: root, stdio: 'inherit', shell: false });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
};
cargo(['run', '--locked', '-p', 'od-tools', '--', 'types']);
if (process.argv.includes('--types-only')) process.exit(0);
if (release) {
  // 发布目录必须自带邮件资源；只在构建机读取参考仓和模型缓存。
  cargo(['run', '--locked', '-p', 'od-tools', '--', 'mail-bundle', '--reference',
    process.env.ZZZ_AUTO_REFERENCE ?? path.join(root, '../ZenlessZoneZero-OneDragon')]);
}
// 咖啡自动路线共用地图资源，开发启动也需要准备。
cargo(['run', '--locked', '-p', 'od-tools', '--', 'map-bundle', '--reference',
  process.env.ZZZ_AUTO_REFERENCE ?? path.join(root, '../ZenlessZoneZero-OneDragon')]);
// 咖啡试运行依赖新提供的模板与固定 Pipeline，开发启动也同步生成。
cargo(['run', '--locked', '-p', 'od-tools', '--', 'coffee-run-bundle', '--reference',
  process.env.ZZZ_AUTO_REFERENCE ?? path.join(root, '../ZenlessZoneZero-OneDragon')]);
cargo(['run', '--locked', '-p', 'od-tools', '--', 'basic-daily-bundle', '--reference',
  process.env.ZZZ_AUTO_REFERENCE ?? path.join(root, '../ZenlessZoneZero-OneDragon')]);
cargo(['run', '--locked', '-p', 'od-tools', '--', 'lost-void-scene-bundle', '--reference',
  process.env.ZZZ_AUTO_REFERENCE ?? path.join(root, '../ZenlessZoneZero-OneDragon')]);
cargo(['run', '--locked', '-p', 'od-tools', '--', 'suibian-temple-bundle', '--reference',
  process.env.ZZZ_AUTO_REFERENCE ?? path.join(root, '../ZenlessZoneZero-OneDragon')]);
cargo(['run', '--locked', '-p', 'od-tools', '--', 'world-patrol-bundle', '--reference',
  process.env.ZZZ_AUTO_REFERENCE ?? path.join(root, '../ZenlessZoneZero-OneDragon')]);
cargo(['run', '--locked', '-p', 'od-tools', '--', 'combat-bundle', '--reference',
  process.env.ZZZ_AUTO_REFERENCE ?? path.join(root, '../ZenlessZoneZero-OneDragon')]);
// 体力试运行共用模拟训练的编队与恢复资源；模拟室消费仍由整组预检阻挡。
for (const command of ['compendium', 'simulation', 'simulation-deploy', 'restore-charge',
  'charge-navigation', 'charge-transport', 'area-patrol', 'expert-challenge', 'deep-hunt',
  'charge-level', 'agent-transport', 'agent-goal', 'ether-synthesis-entry', 'ether-synthesis']) {
  cargo(['run', '--locked', '-p', 'od-tools', '--', `${command}-bundle`, '--reference',
    process.env.ZZZ_AUTO_REFERENCE ?? path.join(root, '../ZenlessZoneZero-OneDragon')]);
}
cargo(['build', '--locked', '-p', 'zzz-engine', ...(release ? ['--release'] : [])]);
if (release) {
  const check = spawnSync(path.join(targetDir, 'release/zzz-engine.exe'),
    ['--root', root, 'mail-bundle-check'], { cwd: root, stdio: 'inherit', shell: false });
  if (check.error) throw check.error;
  if (check.status !== 0) process.exit(check.status ?? 1);
  const mapCheck = spawnSync(path.join(targetDir, 'release/zzz-engine.exe'),
    ['--root', root, 'map-bundle-check', '--bundle', path.join(root, 'resources/generated/map-preview')], { cwd: root, stdio: 'inherit', shell: false });
  if (mapCheck.error) throw mapCheck.error;
  if (mapCheck.status !== 0) process.exit(mapCheck.status ?? 1);
  const coffeeCheck = spawnSync(path.join(targetDir, 'release/zzz-engine.exe'),
    ['--root', root, 'coffee-run-bundle-check', '--bundle', path.join(root, 'resources/generated/coffee-run')], { cwd: root, stdio: 'inherit', shell: false });
  if (coffeeCheck.error) throw coffeeCheck.error;
  if (coffeeCheck.status !== 0) process.exit(coffeeCheck.status ?? 1);
  const dailyCheck = spawnSync(path.join(targetDir, 'release/zzz-engine.exe'),
    ['--root', root, 'basic-daily-bundle-check', '--bundle', path.join(root, 'resources/generated/basic-daily')], { cwd: root, stdio: 'inherit', shell: false });
  if (dailyCheck.error) throw dailyCheck.error;
  if (dailyCheck.status !== 0) process.exit(dailyCheck.status ?? 1);
  const templeCheck = spawnSync(path.join(targetDir, 'release/zzz-engine.exe'),
    ['--root', root, 'suibian-temple-bundle-check', '--bundle', path.join(root, 'resources/generated/suibian-temple')], { cwd: root, stdio: 'inherit', shell: false });
  if (templeCheck.error) throw templeCheck.error;
  if (templeCheck.status !== 0) process.exit(templeCheck.status ?? 1);
  const patrolCheck = spawnSync(path.join(targetDir, 'release/zzz-engine.exe'),
    ['--root', root, 'world-patrol-bundle-check', '--bundle', path.join(root, 'resources/generated/world-patrol')], { cwd: root, stdio: 'inherit', shell: false });
  if (patrolCheck.error) throw patrolCheck.error;
  if (patrolCheck.status !== 0) process.exit(patrolCheck.status ?? 1);
  const combatCheck = spawnSync(path.join(targetDir, 'release/zzz-engine.exe'),
    ['--root', root, 'combat-bundle-check', '--bundle', path.join(root, 'resources/generated/combat')], { cwd: root, stdio: 'inherit', shell: false });
  if (combatCheck.error) throw combatCheck.error;
  if (combatCheck.status !== 0) process.exit(combatCheck.status ?? 1);
  const lostVoidCheck = spawnSync(path.join(targetDir, 'release/zzz-engine.exe'),
    ['--root', root, 'lost-void-resources-check'], { cwd: root, stdio: 'inherit', shell: false });
  if (lostVoidCheck.error) throw lostVoidCheck.error;
  if (lostVoidCheck.status !== 0) process.exit(lostVoidCheck.status ?? 1);
}
const destination = path.join(root, 'apps/desktop/src-tauri/binaries');
mkdirSync(destination, { recursive: true });
copyFileSync(path.join(targetDir, `${release ? 'release' : 'debug'}/zzz-engine.exe`), path.join(destination, 'zzz-auto-engine-x86_64-pc-windows-msvc.exe'));
