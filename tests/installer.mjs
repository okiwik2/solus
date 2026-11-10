// Installer, upgrade and uninstall test. Installs into a temporary folder (per-user, no admin),
// launches the installed copy, upgrades over the top, then uninstalls and checks it cleaned up.
// usage: node tests/installer.mjs dist\SolusSetup-1.0.0.exe
import { execFileSync, spawn } from 'child_process';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { connect } from '../tools/auto.mjs';

const setup = path.resolve(process.argv[2] || 'dist/SolusSetup-1.0.0.exe');
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'solus-inst-'));
const dir = path.join(work, 'Solus');
const data = path.join(work, 'data');
const results = [];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const reg = (key, value) => { try { return execFileSync('reg', ['query', key, ...(value === undefined ? [] : value === '' ? ['/ve'] : ['/v', value])], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }); } catch { return null; } };
async function test(name, fn) { try { await fn(); results.push({ name, ok: true }); console.log(`  PASS  ${name}`); } catch (e) { results.push({ name, ok: false, error: e.message }); console.log(`  FAIL  ${name}: ${e.message}`); } }
const ok = (c, m) => { if (!c) throw new Error(m); };
const run = (exe, args) => execFileSync(exe, args, { stdio: 'ignore', timeout: 300000 });

// A real per-user install shares the Start-menu shortcut and browser registration with the test
// install, and the uninstall step removes them. If one exists it is restored at the end by
// installing the setup under test into the real folder (which upgrades it to that version).
const realKey = 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{6B1F2C1E-5A37-4C2B-9C1D-50D7A5B2E0A1}_is1';
const realDir = ((reg(realKey, 'InstallLocation') || '').match(/REG_SZ\s+(.+?)\\?\s*$/m) || [])[1] || '';
console.log(`Installer test\n  setup: ${setup}\n  into:  ${dir}\n${realDir ? `  note:  Solus is installed at ${realDir}; it will be re-installed from this setup at the end\n` : ''}`);

await test('silent-install', async () => {
  run(setup, ['/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', `/DIR=${dir}`, `/LOG=${path.join(work, 'install.log')}`]);
  for (const f of ['solus.exe', 'ui/chrome/index.html', 'ui/pages/settings.html', 'lists/easylist.txt', 'THIRD-PARTY-NOTICES.txt', 'unins000.exe']) ok(fs.existsSync(path.join(dir, f)), `missing ${f}`);
});

await test('start-menu-shortcut', async () => {
  const lnk = path.join(process.env.APPDATA, 'Microsoft/Windows/Start Menu/Programs/Solus.lnk');
  ok(fs.existsSync(lnk), 'no Start menu shortcut');
});

await test('registered-as-browser', async () => {
  ok(reg('HKCU\\Software\\RegisteredApplications', 'Solus'), 'not in RegisteredApplications');
  const cmd = reg('HKCU\\Software\\Classes\\SolusURL\\shell\\open\\command', '');
  ok(cmd && cmd.includes(dir), 'URL handler does not point at the install folder');
  ok(reg('HKCU\\Software\\Clients\\StartMenuInternet\\Solus\\Capabilities\\URLAssociations', 'https'), 'https association missing');
});

await test('no-autostart', async () => {
  const run = reg('HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run');
  ok(!run || !/solus/i.test(run), 'Solus added itself to Windows start-up');
});

await test('installed-copy-runs-and-browses', async () => {
  const token = 'inst-' + Math.random().toString(36).slice(2) + 'xxxxxxxxxxxxxxxx';
  const port = 47700 + Math.floor(Math.random() * 100);
  const p = spawn(path.join(dir, 'solus.exe'), [`--user-data-dir=${data}`, `--automation-port=${port}`, 'https://example.com/'], { env: { ...process.env, SOLUS_AUTOMATION_TOKEN: token }, stdio: 'ignore' });
  let c;
  for (let i = 0; i < 60; i++) { try { c = connect(port, token); await c.ready; break; } catch { await sleep(250); } }
  ok(c, 'could not connect');
  let title = '';
  for (let i = 0; i < 60 && title !== 'Example Domain'; i++) {
    await sleep(500);
    const s = await c.send({ cmd: 'state' });
    title = (s.windows[0] && s.windows[0].tabs.find((t) => t.id === s.windows[0].active) || {}).title;
  }
  ok(title === 'Example Domain', 'page did not load in the installed build: ' + title);
  const lists = (await c.send({ cmd: 'bridge', name: 'privacy.lists', args: {} })).result.data;
  ok(lists.engineReady, 'bundled filter lists were not loaded');
  await c.send({ cmd: 'quit' });
  c.close();
  await new Promise((r) => { p.on('exit', r); setTimeout(r, 10000); });
});

await test('upgrade-over-existing-install', async () => {
  const before = fs.statSync(path.join(dir, 'solus.exe')).mtimeMs;
  run(setup, ['/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', `/DIR=${dir}`]);
  ok(fs.existsSync(path.join(dir, 'solus.exe')), 'solus.exe missing after upgrade');
  ok(fs.existsSync(path.join(data, 'Profiles', 'default', 'solus.db')), 'user data lost during upgrade');
  void before;
});

await test('uninstall-cleans-up', async () => {
  run(path.join(dir, 'unins000.exe'), ['/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART']);
  await sleep(3000);
  ok(!fs.existsSync(path.join(dir, 'solus.exe')), 'solus.exe still present');
  ok(!reg('HKCU\\Software\\RegisteredApplications', 'Solus'), 'browser registration not removed');
  ok(!reg('HKCU\\Software\\Classes\\SolusURL'), 'URL class not removed');
  ok(!fs.existsSync(path.join(process.env.APPDATA, 'Microsoft/Windows/Start Menu/Programs/Solus.lnk')), 'shortcut not removed');
  ok(fs.existsSync(path.join(data, 'Profiles')), 'silent uninstall must keep user data');
});

if (realDir) {
  run(setup, ['/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', `/DIR=${realDir}`]);
  console.log('  restored the existing install at ' + realDir);
}
const passed = results.filter((r) => r.ok).length;
console.log(`\n${passed}/${results.length} passed`);
fs.writeFileSync(path.join('tests', 'installer-results.json'), JSON.stringify({ when: new Date().toISOString(), setup, results }, null, 2));
try { fs.rmSync(work, { recursive: true, force: true }); } catch {}
process.exit(passed === results.length ? 0 : 1);
