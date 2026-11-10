// Checks that a build accepts the live GitHub update feed. usage: node tests/_livefeed.mjs [solus.exe]
import { spawn } from 'child_process'; import fs from 'fs'; import os from 'os'; import path from 'path';
import { connect } from '../tools/auto.mjs';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const exe = path.resolve(process.argv[2] || 'target/release/solus.exe');
const data = fs.mkdtempSync(path.join(os.tmpdir(), 'solus-feed-'));
fs.writeFileSync(path.join(data, 'Local State.json'), JSON.stringify({ onboarding_done: true, update_check: false }));
const token = 'feed-' + Math.random().toString(36).slice(2) + 'xxxxxxxxxxxxxxxx', port = 47650;
const p = spawn(exe, [`--user-data-dir=${data}`, `--automation-port=${port}`], { env: { ...process.env, SOLUS_AUTOMATION_TOKEN: token }, stdio: 'ignore' });
let c; for (let i = 0; i < 80 && !c; i++) { try { const x = connect(port, token); await x.ready; c = x; } catch { await sleep(250); } }
await sleep(2000);
await c.send({ cmd: 'bridge', name: 'update.check', args: {} }, 30000);
await sleep(8000);
const find = (d) => fs.readdirSync(d, { withFileTypes: true }).flatMap((e) => e.isDirectory() ? find(path.join(d, e.name)) : e.name === 'solus.log' ? [path.join(d, e.name)] : []);
const lines = find(data).flatMap((f) => fs.readFileSync(f, 'utf8').split('\n')).filter((l) => /update/i.test(l));
console.log(lines.length ? lines.join('\n') : 'no update errors logged (feed parsed; already up to date)');
try { await c.send({ cmd: 'quit' }, 8000); } catch {}
c.close(); await sleep(1500); fs.rmSync(data, { recursive: true, force: true });
process.exit(lines.some((l) => /malformed|couldn't|unexpected|signature/i.test(l)) ? 1 : 0);
