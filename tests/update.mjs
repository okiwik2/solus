// Update test: a correctly signed update is downloaded and verified; a tampered one is refused.
// usage: node tests/update.mjs [solus.exe] [installer.exe]
import { spawn, execFileSync } from 'child_process';
import http from 'http';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { connect } from '../tools/auto.mjs';

const exe = path.resolve(process.argv[2] || 'target/release/solus.exe');
const installer = path.resolve(process.argv[3] || 'dist/SolusSetup-1.0.0.exe');
const signer = path.resolve('target/release/solus-sign.exe');
const key = path.join(process.env.APPDATA, 'Solus Release', 'signing.key');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const results = [];
const ok = (c, m) => { if (!c) throw new Error(m); };

let manifest = '';
let payload = fs.readFileSync(installer);
const srv = http.createServer((q, r) => {
  if (q.url === '/latest.json') { r.writeHead(200, { 'Content-Type': 'application/json' }); r.end(manifest); return; }
  if (q.url === '/SolusSetup-9.9.9.exe') { r.writeHead(200); r.end(payload); return; }
  r.writeHead(404); r.end();
});
await new Promise((r) => srv.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${srv.address().port}`;
const sign = () => JSON.parse(execFileSync(signer, ['manifest', key, installer, '9.9.9', `${base}/SolusSetup-9.9.9.exe`, 'test'], { encoding: 'utf8' }));

async function scenario(name, prepare, expect) {
  prepare();
  const data = fs.mkdtempSync(path.join(os.tmpdir(), 'solus-upd-'));
  fs.writeFileSync(path.join(data, 'Local State.json'), JSON.stringify({ onboarding_done: true, update_check: false }));
  const token = 'upd-' + Math.random().toString(36).slice(2) + 'xxxxxxxxxxxxxxxx';
  const port = 47600 + Math.floor(Math.random() * 100);
  const p = spawn(exe, [`--user-data-dir=${data}`, `--automation-port=${port}`], { env: { ...process.env, SOLUS_AUTOMATION_TOKEN: token, SOLUS_UPDATE_FEED: `${base}/latest.json` }, stdio: 'ignore' });
  let c;
  for (let i = 0; i < 60; i++) { try { c = connect(port, token); await c.ready; const s = await c.send({ cmd: 'state' }); if (s.windows.length && s.windows[0].uiReady) break; } catch { await sleep(250); } }
  try {
    await c.send({ cmd: 'bridge', name: 'update.check', args: {} });
    let st = null;
    for (let i = 0; i < 40; i++) { await sleep(500); st = (await c.send({ cmd: 'bridge', name: 'update.status', args: {} })).result.data; if (st.ready) break; }
    expect(st, data);
    results.push({ name, ok: true }); console.log(`  PASS  ${name}`);
  } catch (e) { results.push({ name, ok: false, error: e.message }); console.log(`  FAIL  ${name}: ${e.message}`); }
  try { await c.send({ cmd: 'quit' }, 8000); } catch {}
  c.close();
  await new Promise((r) => { p.on('exit', r); setTimeout(r, 8000); });
  const updDir = path.join(data, 'Updates');
  const leftover = fs.existsSync(updDir) ? fs.readdirSync(updDir) : [];
  try { fs.rmSync(data, { recursive: true, force: true }); } catch {}
  return leftover;
}

console.log('Update test');
await scenario('signed-update-is-accepted', () => { manifest = JSON.stringify(sign()); payload = fs.readFileSync(installer); }, (st) => ok(st.ready && st.ready.version === '9.9.9', 'update not marked ready: ' + JSON.stringify(st)));
await scenario('tampered-installer-is-refused', () => { manifest = JSON.stringify(sign()); payload = Buffer.from(fs.readFileSync(installer)); payload[payload.length - 100] ^= 0xff; }, (st) => ok(!st.ready, 'tampered update was accepted'));
await scenario('forged-signature-is-refused', () => { const m = sign(); m.signature = Buffer.alloc(64, 1).toString('base64'); manifest = JSON.stringify(m); payload = fs.readFileSync(installer); }, (st) => ok(!st.ready, 'forged signature accepted'));
await scenario('foreign-download-host-is-refused', () => { const m = sign(); m.url = 'http://evil.example/SolusSetup.exe'; manifest = JSON.stringify(m); }, (st) => ok(!st.ready, 'update from an untrusted host accepted'));
srv.close();
const passed = results.filter((r) => r.ok).length;
console.log(`\n${passed}/${results.length} passed`);
fs.writeFileSync(path.join('tests', 'update-results.json'), JSON.stringify({ when: new Date().toISOString(), results }, null, 2));
process.exit(passed === results.length ? 0 : 1);
