// Сборка песочницы: Rust → WebAssembly (80 КБ), страница и скрипт, version.json с коммитом запуска.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';

execFileSync('cargo', ['build', '--profile', 'release-wasm', '--target', 'wasm32-unknown-unknown', '--lib'], { stdio: 'inherit', shell: process.platform === 'win32' });

rmSync('site', { recursive: true, force: true });
mkdirSync('site');
copyFileSync('target/wasm32-unknown-unknown/release-wasm/byo_regex.wasm', 'site/byo_regex.wasm');
copyFileSync('site-src/index.html', 'site/index.html');
copyFileSync('site-src/app.js', 'site/app.js');

let commit = process.env.GITHUB_SHA;
if (!commit) {
  try {
    commit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
  } catch {
    commit = undefined;
  }
}
const toml = readFileSync('Cargo.toml', 'utf8');
const version = /^version\s*=\s*"([^"]+)"/m.exec(toml)?.[1];
if (commit) writeFileSync('site/version.json', JSON.stringify({ commit, version, builtAt: new Date().toISOString() }, null, 2) + '\n');
console.log('site/ готов');
