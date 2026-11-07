import { execFileSync } from 'node:child_process';
import { chmodSync, copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// Tauri externalBin expects a target-qualified filename. Stage the same
// compiled helper used during development for the application signing step.
if (process.platform === 'darwin') {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');
  const architecture = process.arch === 'arm64' ? 'aarch64' : 'x86_64';
  const target = process.env.TAURI_ENV_TARGET_TRIPLE || `${architecture}-apple-darwin`;
  if (!['aarch64-apple-darwin', 'x86_64-apple-darwin'].includes(target)) {
    throw new Error(`Unsupported macOS helper target ${target}`);
  }
  const output = execFileSync('cargo', ['build', '-p', 'solar-tools', '--target', target, '--message-format=json'], {
    cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, stdio: ['ignore', 'pipe', 'inherit'],
  });
  const result = output.split('\n').filter(Boolean).map(line => JSON.parse(line)).findLast(
    event => event.reason === 'build-script-executed' && event.package_id.includes('solar-tools'),
  );
  const source = result && join(result.out_dir, 'solar-native-helper');
  if (!source || !existsSync(source)) throw new Error('The macOS native helper was not built');
  const directory = join(root, 'src-tauri', 'binaries');
  mkdirSync(directory, { recursive: true });
  const destination = join(directory, `solar-native-helper-${target}`);
  copyFileSync(source, destination);
  chmodSync(destination, 0o755);
  console.log(`Prepared native helper for ${target}`);
}
