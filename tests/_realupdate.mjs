// Real-world update check: run an installed Solus against the live GitHub feed, let it download and
// verify the newest release, quit, and confirm the installed copy was upgraded.
// usage: node tests/_realupdate.mjs [installed solus.exe]
import { spawn, execFileSync } from 'child_process'; import fs from 'fs'; import os from 'os'; import path from 'path';
import { connect } from '../tools/auto.mjs';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const exe = process.argv[2] || path.join(process.env.LOCALAPPDATA, 'Programs', 'Solus', 'solus.exe');
const ver = () => execFileSync('powershell', ['-NoProfile', '-Command', `(Get-Item '${exe}').VersionInfo.ProductVersion`], { encoding: 'utf8' }).trim();
console.log('installed before:', ver());
const data = fs.mkdtempSync(path.join(os.tmpdir(), 'solus-realupd-'));
fs.writeFileSync(path.join(data, 'Local State.json'), JSON.stringify({ onboarding_done: true, update_check: false }));
const token = 'upd-' + Math.random().toString(36).slice(2) + 'xxxxxxxxxxxxxxxx', port = 47720;
spawn(exe, [`--user-data-dir=${data}`, `--automation-port=${port}`], { env: { ...process.env, SOLUS_AUTOMATION_TOKEN: token }, stdio: 'ignore' });
let c; for (let i = 0; i < 80 && !c; i++) { try { const x = connect(port, token); await x.ready; c = x; } catch { await sleep(250); } }
await sleep(2000);
c.send({ cmd: 'bridge', name: 'update.check', args: {} }, 120000).catch(() => {});
let st;
for (let i = 0; i < 120; i++) { await sleep(1000); st = (await c.send({ cmd: 'bridge', name: 'update.status', args: {} })).result.data; if (st.ready) break; }
console.log('status:', JSON.stringify(st.ready || st));
if (!st.ready) process.exit(1);
try { await c.send({ cmd: 'quit' }, 8000); } catch {}
c.close();
let v = '';
for (let i = 0; i < 60; i++) { await sleep(2000); try { v = ver(); } catch {} if (v === st.ready.version) break; }
console.log('installed after:', v);
try { fs.rmSync(data, { recursive: true, force: true }); } catch {}
process.exit(v === st.ready.version ? 0 : 1);
