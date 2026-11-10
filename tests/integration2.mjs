// More end-to-end tests: third-party cookie blocking, profiles, reminders, extensions, task manager.
// usage: node tests/integration2.mjs [solus.exe]
import { spawn } from 'child_process';
import http from 'http';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { connect } from '../tools/auto.mjs';

const exe = path.resolve(process.argv.slice(2).find((a) => a.endsWith('.exe')) || 'target/debug/solus.exe');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const results = [];
const ok = (c, m) => { if (!c) throw new Error(m); };

const srv = http.createServer((q, r) => {
  const u = q.url.split('?')[0];
  r.writeHead(200, { 'Content-Type': 'text/html' });
  if (u === '/3pc') r.end(`<!doctype html><title>3pc</title><script>window.__tp='pending';addEventListener('message',e=>window.__tp=e.data)</script><iframe src="http://127.0.0.1:${srv.address().port}/frame"></iframe>`);
  else if (u === '/frame') r.end(`<!doctype html><script>document.cookie="tp=1; SameSite=None; Secure; path=/";parent.postMessage("cookie:"+document.cookie,"*")</script>`);
  else if (u === '/1p') r.end(`<!doctype html><title>1p</title><script>document.cookie="fp=1; path=/";window.__fp=document.cookie</script>`);
  else r.end('<!doctype html><title>ok</title>ok');
});
await new Promise((r) => srv.listen(0, '127.0.0.1', r));
const SITE = `http://localhost:${srv.address().port}`;

async function session(seed, fn) {
  const data = fs.mkdtempSync(path.join(os.tmpdir(), 'solus-it2-'));
  fs.writeFileSync(path.join(data, 'Local State.json'), JSON.stringify({ onboarding_done: true }));
  if (seed) { fs.mkdirSync(path.join(data, 'Profiles', 'default'), { recursive: true }); fs.writeFileSync(path.join(data, 'Profiles', 'default', 'settings.json'), JSON.stringify(seed)); }
  const token = 'it2-' + Math.random().toString(36).slice(2) + 'xxxxxxxxxxxxxxxx';
  const port = 47500 + Math.floor(Math.random() * 100);
  const p = spawn(exe, [`--user-data-dir=${data}`, `--automation-port=${port}`], { env: { ...process.env, SOLUS_AUTOMATION_TOKEN: token, SOLUS_ENGINE_ARGS: '--disable-features=CalculateNativeWinOcclusion' }, stdio: 'ignore' });
  let c;
  for (let i = 0; i < 80 && !c; i++) { try { const x = connect(port, token); await x.ready; c = x; } catch { await sleep(250); } }
  if (!c) throw new Error('could not connect to Solus');
  let up = false;
  for (let i = 0; i < 80 && !up; i++) { const st = await c.send({ cmd: 'state' }); up = st.windows.length > 0 && st.windows[0].uiReady; if (!up) await sleep(250); }
  if (!up) throw new Error('Solus UI never became ready');
  const api = {
    c, data,
    bridge: async (name, args = {}, win) => { const r = await c.send({ cmd: 'bridge', name, args, ...(win ? { win } : {}) }); if (!r.result.ok) throw new Error(`${name}: ${r.result.error}`); return r.result.data; },
    state: async () => (await c.send({ cmd: 'state' })).windows,
    eval: async (script, target = 'active') => (await c.send({ cmd: 'eval', target, script })).result,
    go: async (input) => { await api.bridge('navigate', { input }); await sleep(400); for (let i = 0; i < 60; i++) { await sleep(300); const w = (await api.state())[0]; const t = w.tabs.find((x) => x.id === w.active); const norm = (u) => u.split('?')[0].replace(/\/$/, ''); if (!t.loading && norm(t.url) === norm(input)) return t; } },
  };
  try { await fn(api); } finally {
    try { await c.send({ cmd: 'quit' }, 8000); } catch {}
    c.close();
    await new Promise((r) => { p.on('exit', r); setTimeout(r, 8000); });
    try { fs.rmSync(data, { recursive: true, force: true }); } catch {}
  }
}
const only = (process.argv.find((a) => a.startsWith('--only=')) || '').slice(7);
async function test(name, seed, fn) {
  if (only && !name.includes(only)) return;
  try { await session(seed, fn); results.push({ name, ok: true }); console.log(`  PASS  ${name}`); }
  catch (e) { results.push({ name, ok: false, error: e.message }); console.log(`  FAIL  ${name}: ${e.message}`); }
}

console.log(`Solus integration tests (part 2)\n  exe: ${exe}\n`);

async function poll(s, script, okFn, ms = 8000) { let v; const end = Date.now() + ms; while (Date.now() < end) { v = await s.eval(script); if (okFn(v)) return v; await sleep(300); } return v; }
// regression: 1.0.0 shipped with the UI webview never made visible (nothing painted) and the page
// area collapsed to 0x0 because hidden grid children shifted rows/columns
await test('ui-is-painted-and-page-area-has-size', null, async (s) => {
  await s.go('https://example.com/');
  let v;
  for (let i = 0; i < 20; i++) {
    v = JSON.parse(await s.eval('JSON.stringify({vis: document.visibilityState, host: document.querySelector("#page-host").getBoundingClientRect(), w: innerWidth, h: innerHeight})', 'ui'));
    if (v.vis === 'visible' && v.host.width > 0 && v.host.height > 0) break;
    await sleep(300);
  }
  ok(v.vis === 'visible', 'browser UI is not visible: ' + v.vis);
  ok(Math.abs(v.host.width - v.w) < 2 && v.host.height > v.h * 0.5, 'page area has the wrong size: ' + JSON.stringify(v.host));
  ok((await s.eval('document.visibilityState')) === 'visible', 'active page is not visible');
  ok(Math.abs((await s.eval('innerHeight')) - v.host.height) < 3, 'page viewport does not match the page area');
});

await test('third-party-cookies-allowed-control', { block_third_party_cookies: false }, async (s) => {
  await s.go(SITE + "/3pc");
  const v = await poll(s, 'String(window.__tp)', (x) => typeof x === 'string' && x.startsWith('cookie:'));
  ok(String(v).includes('tp=1'), 'control run could not set a third-party cookie (test inconclusive): ' + v);
});

await test('third-party-cookies-blocked-by-default', null, async (s) => {
  await s.go(SITE + "/3pc");
  const v = await poll(s, 'String(window.__tp)', (x) => typeof x === 'string' && x.startsWith('cookie:'));
  ok(String(v).startsWith('cookie:'), 'iframe never reported: ' + v);
  ok(!String(v).includes('tp=1'), 'third-party cookie was set: ' + v);
  await s.go(SITE + "/1p");
  const f = await poll(s, 'String(window.__fp)', (x) => typeof x === 'string' && x.includes('fp=1'));
  ok(String(f).includes('fp=1'), 'first-party cookies broken: ' + f);
});

await test('profiles-are-isolated', null, async (s) => {
  await s.go(`${SITE}/?default-profile-marker`);
  const defWin = (await s.state())[0].id;
  const id = await s.bridge('profiles.create', { name: 'Work' });
  let work;
  for (let i = 0; i < 40 && !work; i++) { await sleep(300); work = (await s.state()).find((w) => w.profile === id && w.uiReady); }
  ok(work, 'no window for the new profile');
  await s.bridge('navigate', { input: `${SITE}/?work-profile-marker` }, work.id);
  await sleep(2500);
  const defHist = await s.bridge('history.query', { text: 'work-profile-marker', limit: 5 }, defWin);
  ok(defHist.length === 0, 'work profile history leaked into the default profile');
  const workHist = await s.bridge('history.query', { text: 'work-profile-marker', limit: 5 }, work.id);
  ok(workHist.length === 1, 'work profile did not record its own history');
  ok(fs.existsSync(path.join(s.data, 'Profiles', id, 'solus.db')), 'no separate database for the profile');
});

await test('reminder-fires', null, async (s) => {
  await s.bridge('reminders.save', { title: 'Test reminder', at: Date.now() + 1500 });
  let fired = false;
  for (let i = 0; i < 40 && !fired; i++) { await sleep(500); fired = (await s.bridge('reminders.list')).some((r) => r.title === 'Test reminder' && r.fired); }
  ok(fired, 'reminder did not fire');
  let bars = [];
  for (let i = 0; i < 20 && !bars.some((b) => b.includes('Test reminder')); i++) {
    await sleep(300);
    bars = JSON.parse((await s.eval('JSON.stringify([...document.querySelectorAll(".infobar .msg")].map(x=>x.textContent))', 'ui')) || '[]');
  }
  ok(bars.some((b) => b.includes('Test reminder')), 'no in-window reminder notice: ' + JSON.stringify(bars));
});

await test('chrome-web-store-extension-install', null, async (s) => {
  await s.c.send({ cmd: 'bridge', name: 'ext.install', args: { id: 'https://chromewebstore.google.com/detail/dark-reader/eimadpbcbfnmbkopoojfekhnkhdbieeh' } });
  let ext;
  for (let i = 0; i < 90 && !ext; i++) { await sleep(500); ext = (await s.bridge('ext.list')).find((x) => /dark reader/i.test(x.name)); }
  ok(ext, 'Dark Reader did not install');
  ok(ext.enabled && ext.version, 'not enabled');
  await s.c.send({ cmd: 'bridge', name: 'ext.remove', args: { id: ext.id } });
  for (let i = 0; i < 20; i++) { await sleep(300); if (!(await s.bridge('ext.list')).length) break; }
  ok(!(await s.bridge('ext.list')).length, 'remove failed');
});

await test('task-manager-maps-processes-to-tabs', null, async (s) => {
  await s.go('https://example.com/');
  await s.bridge('newTab', { url: 'solus://taskmanager/' });
  let rows = 0;
  for (let i = 0; i < 30 && !(rows >= 2); i++) { await sleep(500); rows = await s.eval('document.querySelectorAll("#tabs tr").length'); }
  ok(rows >= 2, 'task manager shows no tabs');
  const mem = await s.eval('[...document.querySelectorAll("#tabs tr")].map(r=>r.textContent).join("|")');
  ok(/(Example Domain|example\.com)[^|]*\d+(\.\d)? MB/.test(mem), 'no memory figure for the example tab: ' + mem.slice(0, 200));
});

srv.close();
const passed = results.filter((r) => r.ok).length;
console.log(`\n${passed}/${results.length} passed`);
fs.writeFileSync(path.join('tests', 'last-results-2.json'), JSON.stringify({ when: new Date().toISOString(), exe, results }, null, 2));
process.exit(passed === results.length ? 0 : 1);
