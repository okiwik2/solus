// Diagnostic: is the real window painting? usage: node tests/_vis.mjs <label> [--no3pc] [--args="..."] [--exe=path]
import { spawn, execFileSync } from 'child_process'; import fs from 'fs'; import os from 'os'; import path from 'path';
import { connect } from '../tools/auto.mjs';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const arg = (k) => (process.argv.find((a) => a.startsWith(`--${k}=`)) || '').slice(k.length + 3);
const label = process.argv[2];
const SP = process.env.SHOT_DIR || os.tmpdir();
const data = fs.mkdtempSync(path.join(os.tmpdir(), 'solus-vis-'));
fs.writeFileSync(path.join(data, 'Local State.json'), JSON.stringify({ onboarding_done: true, update_check: false }));
if (process.argv.includes('--no3pc')) { fs.mkdirSync(path.join(data, 'Profiles', 'default'), { recursive: true }); fs.writeFileSync(path.join(data, 'Profiles', 'default', 'settings.json'), JSON.stringify({ block_third_party_cookies: false })); }
const token = 'vis-' + Math.random().toString(36).slice(2) + 'xxxxxxxxxxxxxxxx', port = 47690 + Math.floor(Math.random() * 9);
const env = { ...process.env, SOLUS_AUTOMATION_TOKEN: token };
if (arg('args')) env.SOLUS_ENGINE_ARGS = arg('args');
const p = spawn(path.resolve(arg('exe') || 'target/release/solus.exe'), [`--user-data-dir=${data}`, `--automation-port=${port}`, 'https://example.com/'], { env, stdio: 'ignore' });
let c; for (let i = 0; i < 80 && !c; i++) { try { const x = connect(port, token); await x.ready; c = x; } catch { await sleep(250); } }
for (let i = 0; i < 40; i++) { const s = await c.send({ cmd: 'state' }); if (s.windows[0]?.uiReady && !s.windows[0].tabs[0].loading) break; await sleep(250); }
await sleep(3000);
const ui = (await c.send({ cmd: 'eval', target: 'ui', script: 'document.visibilityState' })).result;
const tab = (await c.send({ cmd: 'eval', target: 'active', script: 'document.visibilityState' })).result;
const host = (await c.send({ cmd: 'eval', target: 'ui', script: 'JSON.stringify({host:document.querySelector("#page-host").getBoundingClientRect(),cls:document.querySelector("#page-host").className,html:document.querySelector("#page-host").innerHTML.slice(0,200),iw:innerWidth,ih:innerHeight})' })).result;
const tabv = (await c.send({ cmd: 'eval', target: 'active', script: 'innerWidth+"x"+innerHeight' })).result;
console.log('host', host, 'tabViewport', tabv);
try { console.log(execFileSync('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/wintree.ps1', String(p.pid)], { encoding: 'utf8' })); } catch (e) { console.log('wintree failed', e.message.slice(0, 300)); }
const out = path.join(SP, `vis-${label}.png`);
try { execFileSync('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/screen.ps1', '-Out', out], { stdio: 'ignore' }); } catch {}
console.log(`${label}: ui=${ui} tab=${tab} shot=${out}`);
try { await c.send({ cmd: 'quit' }, 8000); } catch {}
c.close(); await sleep(1500); try { fs.rmSync(data, { recursive: true, force: true }); } catch {} process.exit(0);
