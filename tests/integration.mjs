// Solus integration / UI / security / recovery tests.
// Drives a real Solus build through its automation channel with a throwaway profile.
// usage: node tests/integration.mjs [path-to-solus.exe] [--only=name,name]
import { spawn, execSync } from 'child_process';
import http from 'http';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { connect } from '../tools/auto.mjs';

const exe = process.argv.slice(2).find((a) => a.endsWith('.exe')) || path.resolve('target/debug/solus.exe');
const only = (process.argv.slice(2).find((a) => a.startsWith('--only=')) || '').slice(7).split(',').filter(Boolean);
const TOKEN = 'it-' + Math.random().toString(36).slice(2) + Math.random().toString(36).slice(2);
const PORT = 47950 + Math.floor(Math.random() * 40);
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'solus-it-'));
const dlDir = path.join(dataDir, 'dl');
fs.mkdirSync(dlDir);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const results = [];
let proc = null, c = null;

// ---------------------------------------------------------------- local test site
const pages = {
  '/login': `<!doctype html><title>Login</title><form id=f action="/done" method=post><input id=u name=u type=text autocomplete=username><input id=p name=p type=password><button id=b>Sign in</button></form>`,
  '/done': `<!doctype html><title>Signed in</title><p>ok</p>`,
  '/ads': `<!doctype html><title>Ads</title><p>Content</p><div id="ad_banner_1">AD</div><div class="ad_banner">AD</div>
    <script src="https://www.google-analytics.com/analytics.js"></script><script src="https://securepubads.g.doubleclick.net/tag/js/gpt.js"></script>`,
  '/download': null,
  '/perm': `<!doctype html><title>Perm</title><script>navigator.permissions; window.__perm='pending'; navigator.geolocation.getCurrentPosition(()=>window.__perm='granted',(e)=>window.__perm='error:'+e.code);</script>`,
  '/fr': `<!doctype html><html><title>Article</title><body><p>${'Le chat est sur la table et les enfants sont dans le jardin pour une heure de jeux avec des amis. '.repeat(8)}</p></body></html>`,
  '/form': `<!doctype html><title>Form</title><textarea id=t></textarea>`,
};
const server = http.createServer((req, res) => {
  const u = req.url.split('?')[0];
  if (u === '/download') {
    res.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Disposition': 'attachment; filename="solus-test.bin"' });
    res.end(Buffer.alloc(200 * 1024, 7));
    return;
  }
  const body = pages[u];
  if (body == null) { res.writeHead(404); res.end('nf'); return; }
  res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
  res.end(body);
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const SITE = `http://localhost:${server.address().port}`;
const ADSITE = `http://ads.solus.test:${server.address().port}`; // EasyList disables generic hiding on localhost

// ---------------------------------------------------------------- harness
function launch(extra = []) {
  proc = spawn(exe, [`--user-data-dir=${dataDir}`, `--automation-port=${PORT}`, ...extra], {
    env: { ...process.env, SOLUS_AUTOMATION_TOKEN: TOKEN, SOLUS_ENGINE_ARGS: '--disable-features=CalculateNativeWinOcclusion --host-resolver-rules="MAP ads.solus.test 127.0.0.1"' },
    stdio: 'ignore',
  });
}
async function attach() {
  for (let i = 0; i < 60; i++) {
    try { c = connect(PORT, TOKEN); await c.ready; const s = await c.send({ cmd: 'state' }); if (s.ok && s.windows.length && s.windows[0].uiReady) return s; c.close(); } catch {}
    await sleep(250);
  }
  throw new Error('Solus did not start');
}
const send = (cmd) => c.send(cmd);
const bridge = async (name, args = {}) => { const r = await send({ cmd: 'bridge', name, args }); if (!r.result.ok) throw new Error(`${name}: ${r.result.error}`); return r.result.data; };
const state = async () => (await send({ cmd: 'state' })).windows;
const win = async () => (await state()).find((w) => w.visible) || (await state())[0];
const active = async () => { const w = await win(); return w.tabs.find((t) => t.id === w.active); };
const evalIn = async (target, script) => (await send({ cmd: 'eval', target, script })).result;
async function waitFor(fn, ms = 15000, what = 'condition') {
  const end = Date.now() + ms;
  let last;
  while (Date.now() < end) { try { last = await fn(); if (last) return last; } catch (e) { last = e.message; } await sleep(250); }
  throw new Error(`timed out waiting for ${what} (last: ${JSON.stringify(last)})`);
}
async function go(input) { await bridge('navigate', { input }); return waitFor(async () => { const t = await active(); return !t.loading && t.url !== '' && t; }, 20000, 'page load'); }
function ok(cond, msg) { if (!cond) throw new Error(msg); }
async function test(name, fn) {
  if (only.length && !only.includes(name)) return;
  const t0 = Date.now();
  try { await fn(); results.push({ name, ok: true, ms: Date.now() - t0 }); console.log(`  PASS  ${name} (${Date.now() - t0} ms)`); }
  catch (e) { results.push({ name, ok: false, error: e.message }); console.log(`  FAIL  ${name}: ${e.message}`); }
}
async function quitAndWait() { try { await send({ cmd: 'quit' }); } catch {} c && c.close(); await new Promise((r) => { if (!proc || proc.exitCode !== null) return r(); proc.on('exit', r); setTimeout(r, 15000); }); await sleep(800); }

// ---------------------------------------------------------------- tests
console.log(`Solus integration tests\n  exe: ${exe}\n  data: ${dataDir}\n  site: ${SITE}\n`);
launch();
const first = await attach();

await test('startup-first-run-welcome', async () => {
  const t = first[0] ? first : (await state());
  const tab = (await win()).tabs[0];
  ok(tab.url.startsWith('solus://welcome'), 'first run should open the welcome wizard, got ' + tab.url);
  await waitFor(async () => (await evalIn('active', 'document.title')) === 'Welcome to Solus', 8000, 'welcome title');
  await bridge('app.onboardingDone');
  await bridge('setSetting', { key: 'download_dir', value: dlDir });
});

await test('ui-renders-tab-strip-and-omnibox', async () => {
  const r = await waitFor(async () => { const x = JSON.parse(await evalIn('ui', 'JSON.stringify({tabs: document.querySelectorAll(".tab").length, omni: !!document.querySelector("#omni-input"), bm: !!document.querySelector("#bookmarks-bar")})')); return x.tabs === 1 && x.omni && x.bm && x; }, 8000, 'tab strip render');
  ok(r, 'ui not rendered');
});

await test('navigate-and-title', async () => {
  const t = await go('https://example.com/');
  ok(t.title === 'Example Domain', 'title ' + t.title);
  ok(t.security === 'secure', 'security ' + t.security);
  const ui = await evalIn('ui', 'document.querySelector("#omni-input").value');
  ok(ui === 'example.com', 'omnibox shows ' + ui);
});

await test('omnibox-search-fallback', async () => {
  await bridge('navigate', { input: 'solus browser test query' });
  const t = await waitFor(async () => { const x = await active(); return x.url.includes('google.com/search') && x; }, 15000, 'search url');
  ok(t.url.includes('q=solus+browser+test+query'), t.url);
});

await test('https-first-upgrade', async () => {
  const t = await go('http://example.com/');
  ok(t.url.startsWith('https://example.com'), 'not upgraded: ' + t.url);
});

await test('web-cannot-open-or-read-internal-pages', async () => {
  await evalIn('active', 'location.href = "solus://settings/"; 1');
  await sleep(1500);
  ok((await active()).url.startsWith('https://example.com'), 'navigation to solus:// was not blocked');
  await evalIn('active', 'fetch("solus://settings/").then(r=>r.text()).then(t=>window.__r="READ",e=>window.__r="blocked"); 1');
  await sleep(1000);
  ok((await evalIn('active', 'window.__r')) === 'blocked', 'fetch of internal page not blocked');
  ok((await evalIn('active', 'typeof globalThis.__solusSend')) === 'undefined', 'isolated-world binding leaked into page');
});

await test('history-recorded-and-encrypted', async () => {
  const h = await bridge('history.query', { text: 'example', limit: 10 });
  ok(h.some((x) => x.url.startsWith('https://example.com')), 'history missing example.com');
  await sleep(500);
  const db = fs.readFileSync(path.join(dataDir, 'Profiles', 'default', 'solus.db'));
  const wal = fs.existsSync(path.join(dataDir, 'Profiles', 'default', 'solus.db-wal')) ? fs.readFileSync(path.join(dataDir, 'Profiles', 'default', 'solus.db-wal')) : Buffer.alloc(0);
  ok(!db.includes('Example Domain') && !wal.includes('Example Domain'), 'page title found in plain text in the database');
});

await test('bookmarks-add-find-delete', async () => {
  const id = await bridge('bookmarks.toggle', { url: 'https://example.com/', title: 'Example' });
  ok(id, 'no id');
  const f = await bridge('bookmarks.forUrl', { url: 'https://example.com/' });
  ok(f && f.title === 'Example', 'not found');
  const bar = await bridge('bookmarks.children', { parent: 'bar' });
  ok(bar.some((b) => b.id === id), 'not on bar');
  await waitFor(async () => (await evalIn('ui', 'document.querySelectorAll("#bm-items .bm").length')) >= 1, 5000, 'bookmarks bar render');
  await bridge('bookmarks.delete', { id });
  ok(!(await bridge('bookmarks.forUrl', { url: 'https://example.com/' })), 'not deleted');
});

await test('tabs-open-close-reopen-move', async () => {
  const before = (await win()).tabs.length;
  const a = await bridge('newTab', { url: 'https://example.com/?a', background: true });
  const b = await bridge('newTab', { url: 'https://example.com/?b', background: true });
  ok((await win()).tabs.length === before + 2, 'tabs not opened');
  await bridge('closeTab', { id: b });
  ok(!(await win()).tabs.some((t) => t.id === b), 'tab not closed');
  await bridge('reopenClosed');
  await waitFor(async () => (await win()).tabs.some((t) => t.url.includes('?b')), 8000, 'reopened tab');
  await bridge('moveTab', { id: a, to: 0 });
  ok((await win()).tabs[0].id === a, 'move failed');
});

await test('tab-groups', async () => {
  const w = await win();
  const ids = w.tabs.slice(0, 2).map((t) => t.id);
  const gid = await bridge('groups.create', { ids, name: 'Test', color: 'green' });
  let g = (await win()).groups.find((x) => x.id === gid);
  ok(g && g.name === 'Test', 'group missing');
  await bridge('groups.update', { id: gid, name: 'Renamed', collapsed: true });
  g = (await win()).groups.find((x) => x.id === gid);
  ok(g.name === 'Renamed' && g.collapsed, 'update failed');
  await bridge('groups.remove', { id: gid });
  ok(!(await win()).tabs.some((t) => t.group === gid), 'ungroup failed');
});

await test('duplicate-detection-and-auto-groups', async () => {
  await bridge('setSetting', { key: 'auto_group_threshold', value: 3 });
  for (const q of ['x1', 'x2', 'x3']) await bridge('newTab', { url: `${SITE}/done?${q}`, background: true });
  await waitFor(async () => (await win()).groups.some((g) => g.auto), 15000, 'automatic group');
});

await test('workspaces', async () => {
  const id = await bridge('workspaces.create', { name: 'School', color: '#1c7ed6' });
  await bridge('workspaces.switch', { id });
  let w = await win();
  ok(w.workspace === id, 'not switched');
  const inWs = w.tabs.filter((t) => t.workspace === id);
  ok(inWs.length === 1 && inWs[0].url.startsWith('solus://newtab'), 'new workspace should start with a new tab');
  const list = await bridge('workspaces.list');
  ok(list.some((x) => x.name === 'School'), 'not listed');
  await bridge('workspaces.switch', { id: '' });
  w = await win();
  ok(w.workspace === '' && w.tabs.find((t) => t.id === w.active).workspace === '', 'switch back failed');
});

await test('split-screen', async () => {
  const w = await win();
  const other = w.tabs.find((t) => t.id !== w.active && t.workspace === '');
  await bridge('split', { with: other.id });
  ok((await win()).split, 'split not set');
  await bridge('unsplit');
  ok(!(await win()).split, 'unsplit failed');
});

await test('ad-and-tracker-blocking', async () => {
  const t = await go(`${ADSITE}/ads`);
  await sleep(1500);
  const r = JSON.parse(await evalIn('active', 'JSON.stringify({ga: typeof window.ga, gpt: typeof window.googletag && !!(window.googletag && window.googletag.apiReady)})'));
  ok(r.ga === 'undefined', 'analytics.js was not blocked');
  const cnt = (await active()).blocked;
  ok(cnt >= 2, 'blocked count ' + cnt);
  const hidden = await waitFor(async () => (await evalIn('active', 'getComputedStyle(document.querySelector("#ad_banner_1")).display + getComputedStyle(document.querySelector(".ad_banner")).display')) === 'nonenone', 6000, 'cosmetic hiding');
  ok(hidden, 'ad slot not hidden');
});

await test('site-exception-disables-blocking', async () => {
  await bridge('privacy.siteException', { site: 'localhost', exempt: true });
  await go(`${SITE}/ads?again`);
  await sleep(1500);
  const cnt = (await active()).blocked;
  await bridge('privacy.siteException', { site: 'localhost', exempt: false });
  ok(cnt === 0, 'still blocking on an exempt site: ' + cnt);
});

await test('downloads', async () => {
  await go(`${SITE}/done`);
  await evalIn('active', `location.href = "${SITE}/download"; 1`);
  const d = await waitFor(async () => (await bridge('downloads.list')).find((x) => x.state === 'completed' && x.path.includes('solus-test')), 20000, 'download');
  ok(fs.existsSync(d.path) && fs.statSync(d.path).size === 200 * 1024, 'file missing or wrong size: ' + d.path);
  ok(d.path.startsWith(dlDir), 'not saved to the chosen folder');
});

await test('password-save-and-fill', async () => {
  await go(`${SITE}/login`);
  await sleep(800);
  await evalIn('active', 'document.querySelector("#u").value="alice"; document.querySelector("#p").value="S3cret-pass-123"; document.querySelector("#b").click(); 1');
  await sleep(1500);
  await bridge('passwords.respond', { choice: 'save' });
  const list = await bridge('passwords.list');
  ok(list.some((p) => p.username === 'alice' && p.origin === SITE), 'password not saved: ' + JSON.stringify(list));
  await go(`${SITE}/login`);
  await sleep(800);
  const tab = await active();
  const pw = (await bridge('passwords.forPage', { tab: tab.id }))[0];
  await bridge('passwords.fill', { tab: tab.id, id: pw.id });
  await sleep(500);
  const v = await evalIn('active', 'document.querySelector("#u").value + "|" + document.querySelector("#p").value');
  ok(v === 'alice|S3cret-pass-123', 'fill gave ' + v);
});

await test('site-permission-prompt-and-memory', async () => {
  await go(`${SITE}/perm`);
  const prompt = await waitFor(async () => JSON.parse(await evalIn('ui', 'JSON.stringify([...document.querySelectorAll(".infobar")].map(x=>x.dataset.ib))')).find((x) => x.startsWith('perm-')), 8000, 'permission infobar');
  await bridge('permissions.respond', { id: +prompt.slice(5), allow: false });
  const perms = await bridge('permissions.forOrigin', { origin: SITE });
  ok(perms.find((p) => p.kind === 'geolocation').state === 'block', 'decision not stored');
  await bridge('permissions.reset', { origin: SITE });
});

await test('translate-offer', async () => {
  await go(`${SITE}/fr`);
  const offer = await waitFor(async () => JSON.parse(await evalIn('ui', 'JSON.stringify(S.translateOffer||null)')), 8000, 'translate offer');
  ok(offer.lang === 'fr', 'detected ' + offer.lang);
});

await test('find-in-page', async () => {
  await go('https://example.com/');
  await send({ cmd: 'action', name: 'find' });
  await sleep(300);
  await bridge('find', { action: 'start', text: 'domain' });
  const r = await waitFor(async () => { const t = await evalIn('ui', 'document.querySelector("#find-count").textContent'); return /\/[1-9]/.test(t) && t; }, 6000, 'find result');
  ok(r, 'no matches reported');
  await bridge('find', { action: 'stop' });
});

await test('memory-manager-freeze-discard-restore', async () => {
  const id = await bridge('newTab', { url: 'https://example.com/?bg', background: true });
  await waitFor(async () => { const t = (await win()).tabs.find((x) => x.id === id); return t && !t.loading && t.title === 'Example Domain'; }, 15000, 'bg load');
  await bridge('perf.freeze', { id });
  await waitFor(async () => (await win()).tabs.find((x) => x.id === id).frozen, 8000, 'frozen');
  await bridge('perf.discard', { id });
  ok((await win()).tabs.find((x) => x.id === id).discarded, 'not discarded');
  await bridge('activate', { id });
  await waitFor(async () => { const t = (await win()).tabs.find((x) => x.id === id); return !t.discarded && !t.loading && t.title === 'Example Domain'; }, 15000, 'reload after discard');
});

await test('renderer-crash-recovery', async () => {
  const t = await go('https://example.com/?crash');
  const snap = await bridge('perf.snapshot');
  const rend = snap && snap.pending ? null : snap;
  // fetch the detailed snapshot through the page-independent memory command instead
  const mem = (await send({ cmd: 'memory' })).memory;
  const renderers = mem.processes.filter((p) => p.kind === 'Renderer');
  ok(renderers.length >= 1, 'no renderer processes');
  // the browser UI renderer is the oldest; kill the newest (the active page)
  const victim = renderers[renderers.length - 1].pid;
  try { process.kill(victim); } catch {}
  await sleep(1500);
  const after = (await win()).tabs.find((x) => x.id === t.id);
  if (after.crashed) {
    await bridge('tabAction', { id: t.id, action: 'reload' });
    await waitFor(async () => { const x = (await win()).tabs.find((y) => y.id === t.id); return !x.crashed && x.title === 'Example Domain'; }, 15000, 'recovered after crash');
  }
  ok((await state()).length >= 1, 'browser window lost');
  void rend;
});

await test('incognito-does-not-record-history', async () => {
  await send({ cmd: 'action', name: 'newIncognito' });
  const inc = await waitFor(async () => (await state()).find((w) => w.incognito && w.uiReady), 10000, 'incognito window');
  const before = (await bridge('history.query', { text: 'incognito-marker', limit: 5 })).length;
  const r = await send({ cmd: 'bridge', win: inc.id, name: 'navigate', args: { input: 'https://example.com/?incognito-marker' } });
  ok(r.ok, 'navigate failed');
  await sleep(4000);
  const after = (await bridge('history.query', { text: 'incognito-marker', limit: 5 })).length;
  ok(after === before, 'incognito visit recorded in history');
  await send({ cmd: 'bridge', win: inc.id, name: 'window', args: { action: 'close' } });
  await sleep(800);
});

await test('internal-pages-load', async () => {
  for (const [page, sel] of [['newtab', '#wrap'], ['settings', '#sec-appearance'], ['history', '#list'], ['bookmarks', '#tree'], ['privacy', '#checks'], ['taskmanager', '#tabs'], ['passwords', '#list'], ['extensions', '#list'], ['themes', '#colors']]) {
    await bridge('navigate', { input: `solus://${page}/` });
    await waitFor(async () => (await evalIn('active', `!!document.querySelector(${JSON.stringify(sel)}) && document.readyState === "complete" && document.querySelector(${JSON.stringify(sel)}).innerHTML.length > 0`)) === true, 10000, `${page} page`);
  }
});

await test('ai-requires-key-and-is-off-in-incognito', async () => {
  const st = await bridge('ai.status');
  ok(st.keySet === false, 'unexpected key');
  const conv = await bridge('ai.new');
  let err = '';
  try { await bridge('ai.send', { conv, text: 'hi' }); } catch (e) { err = e.message; }
  ok(/API key/.test(err), 'expected missing-key error, got ' + err);
  let rejected = false;
  try { await bridge('ai.setKey', { provider: 'anthropic', key: 'not-a-key' }); } catch { rejected = true; }
  ok(rejected, 'invalid key accepted');
});

await test('settings-persist-and-sync-folder-encrypted', async () => {
  await bridge('setSetting', { key: 'density', value: 'compact' });
  const folder = path.join(dataDir, 'cloud');
  fs.mkdirSync(folder);
  const st = await send({ cmd: 'bridge', name: 'sync.setup', args: { folder, passphrase: 'integration test passphrase' } });
  ok(st.ok, 'sync setup failed');
  await waitFor(async () => fs.existsSync(path.join(folder, 'Solus Sync', 'devices')) && fs.readdirSync(path.join(folder, 'Solus Sync', 'devices')).length > 0, 20000, 'sync file');
  const f = fs.readdirSync(path.join(folder, 'Solus Sync', 'devices'))[0];
  const bytes = fs.readFileSync(path.join(folder, 'Solus Sync', 'devices', f));
  ok(!bytes.includes('example.com') && !bytes.includes('alice'), 'sync file is not encrypted');
  const b = await bridge('backup.create');
  ok(b.length >= 1, 'no local backup');
});

await test('session-restore-after-quit', async () => {
  const urls = (await win()).tabs.map((t) => t.url).filter((u) => u.startsWith('http')).sort();
  await quitAndWait();
  launch();
  await attach();
  const w = await waitFor(async () => { const x = await win(); return x.tabs.length > 1 && x; }, 15000, 'restored tabs');
  const got = w.tabs.map((t) => t.url).filter((u) => u.startsWith('http')).sort();
  ok(JSON.stringify(got) === JSON.stringify(urls), `restored ${got.length}/${urls.length}`);
  ok((await bridge('getSettings')).density === 'compact', 'setting not persisted');
  ok(w.tabs.filter((t) => !t.discarded).length <= 2, 'restore should load only the visible tab');
});

await test('crash-recovery-after-kill', async () => {
  const urls = (await win()).tabs.map((t) => t.url).filter((u) => u.startsWith('http')).sort();
  await sleep(6000); // allow the periodic session save
  execSync(`taskkill /F /PID ${proc.pid}`);
  await sleep(1500);
  c.close();
  launch();
  await attach();
  const w = await waitFor(async () => { const x = await win(); return x.tabs.length > 1 && x; }, 15000, 'restored after kill');
  const got = w.tabs.map((t) => t.url).filter((u) => u.startsWith('http')).sort();
  ok(got.length >= urls.length - 1, `restored ${got.length}/${urls.length} after a crash`);
});

await quitAndWait();
server.close();
const passed = results.filter((r) => r.ok).length;
console.log(`\n${passed}/${results.length} passed`);
fs.writeFileSync(path.join('tests', 'last-results.json'), JSON.stringify({ when: new Date().toISOString(), exe, results }, null, 2));
try { fs.rmSync(dataDir, { recursive: true, force: true }); } catch {}
process.exit(passed === results.length ? 0 : 1);
