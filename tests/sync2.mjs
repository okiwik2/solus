// Two-device sync: two Solus instances with separate data folders ("PC A" and "PC B") share one
// sync folder, as two computers sharing a OneDrive/Google Drive folder would.
// usage: node tests/sync2.mjs [solus.exe]
import { spawn } from 'child_process';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { connect } from '../tools/auto.mjs';

const exe = path.resolve(process.argv.slice(2).find((a) => a.endsWith('.exe')) || 'target/debug/solus.exe');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const ok = (c, m) => { if (!c) throw new Error(m); };
const results = [];
const PASS = 'correct horse battery staple';
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'solus-sync2-'));
const cloud = path.join(tmp, 'OneDrive');
fs.mkdirSync(cloud);

async function device(name, port) {
  const data = path.join(tmp, name);
  fs.mkdirSync(data);
  fs.writeFileSync(path.join(data, 'Local State.json'), JSON.stringify({ onboarding_done: true, update_check: false, device_name: name }));
  const token = 'sync-' + Math.random().toString(36).slice(2) + 'xxxxxxxxxxxxxxxx';
  const p = spawn(exe, [`--user-data-dir=${data}`, `--automation-port=${port}`], { env: { ...process.env, SOLUS_AUTOMATION_TOKEN: token }, stdio: 'ignore' });
  let c;
  for (let i = 0; i < 80 && !c; i++) { try { const x = connect(port, token); await x.ready; c = x; } catch { await sleep(250); } }
  if (!c) throw new Error('could not connect to ' + name);
  for (let i = 0; i < 80; i++) { const st = await c.send({ cmd: 'state' }); if (st.windows.length && st.windows[0].uiReady) break; await sleep(250); }
  const call = async (n, args = {}) => { const r = await c.send({ cmd: 'bridge', name: n, args }, 60000); if (!r.result.ok) throw new Error(`${name} ${n}: ${r.result.error}`); return r.result.data; };
  const quit = async () => { try { await c.send({ cmd: 'quit' }, 8000); } catch {} c.close(); await new Promise((r) => { p.on('exit', r); setTimeout(r, 8000); }); };
  return { name, data, call, quit };
}
async function test(name, fn) {
  try { await fn(); results.push({ name, ok: true }); console.log(`  PASS  ${name}`); }
  catch (e) { results.push({ name, ok: false, error: e.message }); console.log(`  FAIL  ${name}: ${e.message}`); }
}
const bmByUrl = async (d, url) => (await d.call('bookmarks.search', { q: url })).find((b) => b.url === url);

console.log(`Solus two-device sync tests\n  exe: ${exe}\n  shared folder: ${cloud}\n`);
const A = await device('PC-A', 47801);
const B = await device('PC-B', 47802);

let noteId;
await test('first-device-creates-an-encrypted-vault', async () => {
  await A.call('bookmarks.add', { parent: 'bar', title: 'Kiwi Bank', url: 'https://kiwibank.example/' });
  noteId = await A.call('notes.save', { url: 'https://kiwibank.example/', body: 'Account ends in 4321' });
  await A.call('tasks.save', { title: 'Renew passport' });
  await A.call('passwords.save', { url: 'https://kiwibank.example/', username: 'omar@example.com', password: 'Tui-bird-2026!' });
  await A.call('setSetting', { key: 'density', value: 'compact' });
  const r = await A.call('sync.setup', { folder: cloud, passphrase: PASS });
  ok(r.created === true && r.firstSync, 'vault not created: ' + JSON.stringify(r));
  const root = path.join(cloud, 'Solus Sync');
  ok(fs.existsSync(path.join(root, 'vault.json')), 'no vault.json');
  const files = fs.readdirSync(path.join(root, 'devices'));
  ok(files.length === 1, 'device files: ' + files);
  const all = Buffer.concat([fs.readFileSync(path.join(root, 'vault.json')), fs.readFileSync(path.join(root, 'devices', files[0]))]);
  for (const secret of ['Tui-bird-2026!', 'omar@example.com', 'kiwibank', '4321', 'Renew passport', PASS]) ok(!all.includes(secret), `"${secret}" readable in the sync folder`);
});

await test('second-device-rejects-the-wrong-passphrase', async () => {
  let err = '';
  try { await B.call('sync.setup', { folder: cloud, passphrase: 'wrong passphrase here' }); } catch (e) { err = e.message; }
  ok(/doesn't match/.test(err), 'wrong passphrase accepted: ' + err);
  ok(!(await B.call('sync.status')).sync.enabled, 'sync enabled after a failed setup');
});

await test('second-device-joins-and-receives-everything', async () => {
  const r = await B.call('sync.setup', { folder: cloud, passphrase: PASS });
  ok(r.created === false && r.firstSync && r.firstSync.merged >= 4, 'join result: ' + JSON.stringify(r));
  ok(await bmByUrl(B, 'https://kiwibank.example/'), 'bookmark missing on B');
  ok((await B.call('notes.list')).some((n) => n.body === 'Account ends in 4321'), 'note missing on B');
  ok((await B.call('tasks.list')).some((t) => t.title === 'Renew passport'), 'task missing on B');
  ok((await B.call('passwords.list')).some((p) => p.username === 'omar@example.com' && p.host === 'kiwibank.example'), 'password missing on B');
  ok((await B.call('getSettings')).density === 'compact', 'setting not synced');
});

await test('edits-and-deletions-flow-back', async () => {
  const bm = await bmByUrl(B, 'https://kiwibank.example/');
  await B.call('bookmarks.update', { id: bm.id, title: 'Kiwibank (renamed on B)' });
  const bNote = (await B.call('notes.list')).find((n) => n.body === 'Account ends in 4321');
  await B.call('notes.delete', { id: bNote.id });
  await B.call('bookmarks.add', { parent: 'bar', title: 'Added on B', url: 'https://b.example/' });
  await B.call('sync.now');
  const r = await A.call('sync.now');
  ok(r.devices.some((d) => d.name === 'PC-B'), 'A does not see PC-B: ' + JSON.stringify(r.devices));
  ok((await bmByUrl(A, 'https://kiwibank.example/')).title === 'Kiwibank (renamed on B)', 'rename did not reach A');
  ok(await bmByUrl(A, 'https://b.example/'), 'new bookmark did not reach A');
  ok(!(await A.call('notes.list')).some((n) => n.body === 'Account ends in 4321'), 'deleted note came back on A');
});

await test('conflicting-edits-latest-wins-on-both', async () => {
  const a = await bmByUrl(A, 'https://b.example/');
  await A.call('bookmarks.update', { id: a.id, title: 'Edited on A first' });
  await sleep(50);
  const b = await bmByUrl(B, 'https://b.example/');
  await B.call('bookmarks.update', { id: b.id, title: 'Edited on B later' });
  await A.call('sync.now'); await B.call('sync.now'); await A.call('sync.now');
  const ta = (await bmByUrl(A, 'https://b.example/')).title, tb = (await bmByUrl(B, 'https://b.example/')).title;
  ok(ta === 'Edited on B later' && tb === 'Edited on B later', `devices disagree: A="${ta}" B="${tb}"`);
});

await test('a-tampered-or-corrupt-device-file-is-ignored-safely', async () => {
  const dir = path.join(cloud, 'Solus Sync', 'devices');
  // corrupt every device file (A skips its own, so B's is the one that matters)
  const before = (await A.call('bookmarks.search', { q: 'example' })).length;
  for (const f of fs.readdirSync(dir)) { const p = path.join(dir, f); const buf = fs.readFileSync(p); buf[buf.length - 5] ^= 0xff; fs.writeFileSync(p, buf); }
  const r = await A.call('sync.now');
  ok(r.merged === 0, 'merged data from a corrupt file');
  ok((await A.call('bookmarks.search', { q: 'example' })).length === before, 'local data changed');
  fs.writeFileSync(path.join(dir, 'garbage.sync'), 'not a sync file');
  await A.call('sync.now');
});

await test('device-restarts-and-keeps-syncing', async () => {
  await B.quit();
  const B2 = await device('PC-B-again', 47803).catch(() => null);
  // a fresh device folder joining an existing vault = a third PC
  ok(B2, 'third device did not start');
  await B2.call('sync.setup', { folder: cloud, passphrase: PASS });
  await A.call('bookmarks.add', { parent: 'bar', title: 'After corruption', url: 'https://after.example/' });
  await A.call('sync.now');
  await B2.call('sync.now');
  ok(await bmByUrl(B2, 'https://after.example/'), 'third device did not receive the new bookmark');
  ok(await bmByUrl(B2, 'https://kiwibank.example/'), 'third device missing older data');
  await B2.quit();
});

await A.quit();
try { fs.rmSync(tmp, { recursive: true, force: true }); } catch {}
const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} passed`);
process.exit(failed.length ? 1 : 0);
