// Visual tour: screenshots of the real window in several UI states. usage: node tests/_tour.mjs [--dark]
import { spawn, execFileSync } from 'child_process'; import fs from 'fs'; import os from 'os'; import path from 'path';
import { connect } from '../tools/auto.mjs';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const SP = process.env.SHOT_DIR || os.tmpdir();
const dark = process.argv.includes('--dark');
const data = fs.mkdtempSync(path.join(os.tmpdir(), 'solus-tour-'));
fs.writeFileSync(path.join(data, 'Local State.json'), JSON.stringify({ onboarding_done: true, update_check: false }));
if (dark) { fs.mkdirSync(path.join(data, 'Profiles', 'default'), { recursive: true }); fs.writeFileSync(path.join(data, 'Profiles', 'default', 'settings.json'), JSON.stringify({ theme: 'dark' })); }
const token = 'tour-' + Math.random().toString(36).slice(2) + 'xxxxxxxxxxxxxxxx', port = 47710;
const p = spawn(path.resolve('target/release/solus.exe'), [`--user-data-dir=${data}`, `--automation-port=${port}`, 'https://www.rust-lang.org/'], { env: { ...process.env, SOLUS_AUTOMATION_TOKEN: token }, stdio: 'ignore' });
let c; for (let i = 0; i < 80 && !c; i++) { try { const x = connect(port, token); await x.ready; c = x; } catch { await sleep(250); } }
for (let i = 0; i < 60; i++) { const s = await c.send({ cmd: 'state' }); if (s.windows[0]?.uiReady && !s.windows[0].tabs[0].loading) break; await sleep(250); }
const ui = async (script) => { const t0 = Date.now(); try { return await c.send({ cmd: 'eval', target: 'ui', script }, 10000); } catch (e) { console.log('  ui eval failed after', Date.now() - t0, 'ms:', script.slice(0, 60)); } };
const bridge = async (name, args = {}) => { try { return await c.send({ cmd: 'bridge', name, args }, 10000); } catch (e) { console.log('  bridge failed:', name); } };
const tag = dark ? 'dark-' : '';
const shot = (name) => { const out = path.join(SP, `tour-${tag}${name}.png`); try { execFileSync('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/screen.ps1', '-Out', out], { stdio: 'ignore' }); } catch {} console.log(out); };
// bigger window so the layout is representative
execFileSync('powershell', ['-NoProfile', '-Command', `Add-Type -Name U -Namespace W -MemberDefinition '[DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h,int x,int y,int w,int h2,bool r);'; $h=(Get-Process -Id ${p.pid}).MainWindowHandle; [W.U]::MoveWindow($h, 60, 40, 1700, 1050, $true)`]);
await sleep(3500); shot('1-page');
await ui('document.querySelector("#menu-btn").click()'); await sleep(900); shot('2-menu');
await ui('document.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape"}));UI.closePopups()'); await sleep(400);
await bridge('newTab', { url: 'solus://newtab/' }); await sleep(2500); shot('3-newtab');
await ui('document.querySelector("#sidebar-btn").click()'); await sleep(1200); shot('4-sidebar');
await ui('document.querySelector("#sidebar-btn").click()'); await sleep(500);
await bridge('newTab', { url: 'solus://settings/' }); await sleep(2000); shot('5-settings');
const st = (await c.send({ cmd: 'state' })).windows[0];
await bridge('activate', { id: st.tabs[0].id }); await sleep(500); await c.send({ cmd: 'action', name: 'splitView', data: { with: st.tabs[1].id } }); await sleep(2500); shot('6-split');
try { await c.send({ cmd: 'quit' }, 8000); } catch {}
c.close(); await sleep(1500); try { fs.rmSync(data, { recursive: true, force: true }); } catch {} process.exit(0);
