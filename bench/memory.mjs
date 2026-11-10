// Memory benchmark: Solus vs Google Chrome under the same workload.
// Each browser runs alone with a fresh profile and default settings; memory is the sum of the
// private working set of the browser's whole process tree (what Task Manager's "Memory" shows).
// usage: node bench/memory.mjs [--minutes=6] [--solus=target\release\solus.exe] [--profile=balanced]
import { spawn, execFileSync } from 'child_process';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { connect } from '../tools/auto.mjs';

const arg = (k, d) => (process.argv.find((a) => a.startsWith(`--${k}=`)) || '').split('=')[1] || d;
const MINUTES = +arg('minutes', 6);
const SOLUS = path.resolve(arg('solus', 'target/release/solus.exe'));
const PERF = arg('profile', 'balanced');
const CHROME = [process.env.ProgramFiles, process.env['ProgramFiles(x86)'], process.env.LOCALAPPDATA].map((p) => p && path.join(p, 'Google/Chrome/Application/chrome.exe')).find((p) => p && fs.existsSync(p));
const SITES = [
  'https://en.wikipedia.org/wiki/New_Zealand', 'https://www.bbc.com/news', 'https://github.com/rust-lang/rust',
  'https://www.youtube.com/', 'https://www.reddit.com/r/newzealand/', 'https://stackoverflow.com/questions',
  'https://www.theguardian.com/international', 'https://www.nytimes.com/', 'https://www.amazon.com/', 'https://www.stuff.co.nz/',
];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const mb = (b) => Math.round(b / 1048576);

function measure(rootPid) {
  const out = execFileSync('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', path.join('bench', 'procmem.ps1'), '-RootPid', String(rootPid)], { encoding: 'utf8' });
  return JSON.parse(out.trim());
}
function killTree(pid) { try { execFileSync('taskkill', ['/F', '/T', '/PID', String(pid)], { stdio: 'ignore' }); } catch {} }

async function runChrome(urls, checkpoints) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'bench-chrome-'));
  const p = spawn(CHROME, [`--user-data-dir=${dir}`, '--no-first-run', '--no-default-browser-check', '--new-window', ...urls], { stdio: 'ignore' });
  const out = await sample(p.pid, checkpoints);
  killTree(p.pid);
  await sleep(2000);
  try { fs.rmSync(dir, { recursive: true, force: true }); } catch {}
  return out;
}

async function runSolus(urls, checkpoints) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'bench-solus-'));
  // fresh profile, onboarding already done, chosen performance profile
  fs.writeFileSync(path.join(dir, 'Local State.json'), JSON.stringify({ onboarding_done: true, perf_profile: PERF, clean_exit: true }));
  const token = 'bench-' + Math.random().toString(36).slice(2) + 'xxxxxxxxxxxx';
  const port = 47800 + Math.floor(Math.random() * 100);
  const p = spawn(SOLUS, [`--user-data-dir=${dir}`, `--automation-port=${port}`, ...urls], { stdio: 'ignore', env: { ...process.env, SOLUS_AUTOMATION_TOKEN: token } });
  const out = await sample(p.pid, checkpoints);
  try { const c = connect(port, token); await c.ready; await c.send({ cmd: 'quit' }, 10000); c.close(); } catch {}
  await sleep(2500);
  killTree(p.pid);
  try { fs.rmSync(dir, { recursive: true, force: true }); } catch {}
  return out;
}

async function sample(pid, checkpoints) {
  const t0 = Date.now();
  const res = [];
  for (const s of checkpoints) {
    const wait = t0 + s * 1000 - Date.now();
    if (wait > 0) await sleep(wait);
    const m = measure(pid);
    res.push({ at: s, mb: mb(m.privateWs), processes: m.count, byType: Object.fromEntries(Object.entries(m.byType || {}).map(([k, v]) => [k, mb(v)])) });
    console.log(`    t=${s}s  ${mb(m.privateWs)} MB in ${m.count} processes`);
  }
  return res;
}

if (!CHROME) { console.error('Google Chrome not found'); process.exit(1); }
const chromeVer = (() => { try { return execFileSync('powershell', ['-NoProfile', '-Command', `(Get-Item '${CHROME}').VersionInfo.ProductVersion`], { encoding: 'utf8' }).trim(); } catch { return '?'; } })();
console.log(`Chrome ${chromeVer}\nSolus  ${SOLUS} (${PERF} profile)\n`);
const report = { when: new Date().toISOString(), chrome: chromeVer, solusProfile: PERF, machine: `${os.cpus()[0].model}, ${Math.round(os.totalmem() / 2 ** 30)} GB RAM`, scenarios: [] };

const idle = [20];
const loaded = [45, 180, MINUTES * 60];
for (const [name, urls, cps] of [['1 tab (new tab page)', [], idle], [`${SITES.length} popular sites`, SITES, loaded]]) {
  console.log(`Scenario: ${name}`);
  console.log('  Chrome'); const c = await runChrome(urls, cps);
  await sleep(4000);
  console.log('  Solus'); const s = await runSolus(urls, cps);
  await sleep(4000);
  report.scenarios.push({ name, chrome: c, solus: s });
}

let md = `# Memory benchmark — ${report.when.slice(0, 10)}\n\n${report.machine}. Chrome ${report.chrome} (fresh profile, default settings) vs Solus (${PERF} profile, fresh profile).\n` +
  `Memory = private working set summed over each browser's whole process tree. Sites: ${SITES.map((u) => new URL(u).hostname).join(', ')}.\n\n` +
  '| Scenario | Time | Chrome | Solus | Difference |\n|---|---|---|---|---|\n';
for (const sc of report.scenarios) {
  sc.chrome.forEach((c, i) => {
    const s = sc.solus[i];
    md += `| ${sc.name} | ${c.at < 60 ? c.at + ' s' : c.at / 60 + ' min'} | ${c.mb} MB (${c.processes} proc) | ${s.mb} MB (${s.processes} proc) | ${s.mb <= c.mb ? '−' : '+'}${Math.abs(Math.round((1 - s.mb / c.mb) * 100))}% |\n`;
  });
}
fs.mkdirSync('bench/results', { recursive: true });
const stamp = report.when.replace(/[:.]/g, '-').slice(0, 19);
fs.writeFileSync(`bench/results/memory-${stamp}.json`, JSON.stringify(report, null, 2));
fs.writeFileSync(`bench/results/memory-${stamp}.md`, md);
console.log('\n' + md);
