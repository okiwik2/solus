// Startup time and background CPU: Solus vs Google Chrome (fresh profiles, default settings).
// usage: node bench/cpu-startup.mjs [--solus=target\release\solus.exe] [--runs=5]
import { execFileSync, spawn } from 'child_process';
import fs from 'fs';
import os from 'os';
import path from 'path';

const arg = (k, d) => (process.argv.find((a) => a.startsWith(`--${k}=`)) || '').split('=')[1] || d;
const SOLUS = path.resolve(arg('solus', 'target/release/solus.exe'));
const RUNS = +arg('runs', 5);
const CHROME = [process.env.ProgramFiles, process.env['ProgramFiles(x86)'], process.env.LOCALAPPDATA].map((p) => p && path.join(p, 'Google/Chrome/Application/chrome.exe')).find((p) => p && fs.existsSync(p));
const SITES = ['https://en.wikipedia.org/wiki/New_Zealand', 'https://www.bbc.com/news', 'https://github.com/rust-lang/rust', 'https://www.youtube.com/', 'https://www.reddit.com/r/newzealand/', 'https://stackoverflow.com/questions', 'https://www.theguardian.com/international', 'https://www.nytimes.com/', 'https://www.amazon.com/', 'https://www.stuff.co.nz/'];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const ps = (file, args) => execFileSync('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', path.join('bench', file), ...args], { encoding: 'utf8' }).trim();
const kill = (pid) => { try { execFileSync('taskkill', ['/F', '/T', '/PID', String(pid)], { stdio: 'ignore' }); } catch {} };
const median = (a) => { const s = [...a].sort((x, y) => x - y); return s[Math.floor(s.length / 2)]; };

function freshDir(kind) {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), `bench-${kind}-`));
  if (kind === 'solus') fs.writeFileSync(path.join(d, 'Local State.json'), JSON.stringify({ onboarding_done: true }));
  return d;
}
function argLine(kind, dir, urls) {
  const q = (s) => `"${s}"`;
  return kind === 'chrome'
    ? [q(`--user-data-dir=${dir}`), '--no-first-run', '--no-default-browser-check', ...urls].join(' ')
    : [q(`--user-data-dir=${dir}`), ...urls].join(' ');
}

async function startup(kind) {
  const times = [];
  for (let i = 0; i < RUNS; i++) {
    const dir = freshDir(kind);
    const [ms, pid] = ps('startup.ps1', ['-Exe', kind === 'chrome' ? CHROME : SOLUS, '-ArgLine', argLine(kind, dir, [])]).split(' ').map(Number);
    times.push(ms);
    await sleep(3000);
    kill(pid);
    await sleep(2000);
    try { fs.rmSync(dir, { recursive: true, force: true }); } catch {}
  }
  return times;
}

async function idleCpu(kind) {
  const dir = freshDir(kind);
  const p = spawn(kind === 'chrome' ? CHROME : SOLUS, kind === 'chrome' ? [`--user-data-dir=${dir}`, '--no-first-run', '--no-default-browser-check', ...SITES] : [`--user-data-dir=${dir}`, ...SITES], { stdio: 'ignore' });
  await sleep(60000); // let pages finish loading
  const a = JSON.parse(ps('procmem.ps1', ['-RootPid', String(p.pid)]));
  await sleep(180000); // three minutes of background activity with no user input
  const b = JSON.parse(ps('procmem.ps1', ['-RootPid', String(p.pid)]));
  kill(p.pid);
  await sleep(2000);
  try { fs.rmSync(dir, { recursive: true, force: true }); } catch {}
  const cores = os.cpus().length;
  return { cpuMs: b.cpuMs - a.cpuMs, avgPercentOfMachine: +(((b.cpuMs - a.cpuMs) / 180000 / cores) * 100).toFixed(2), processes: b.count };
}

console.log(`Chrome: ${CHROME}\nSolus:  ${SOLUS}\n`);
const r = { when: new Date().toISOString(), machine: `${os.cpus()[0].model}, ${os.cpus().length} logical cores` };
console.log('Startup (time until the main window is visible)');
r.chromeStartup = await startup('chrome'); console.log('  Chrome', r.chromeStartup.join(', '), 'ms');
r.solusStartup = await startup('solus'); console.log('  Solus ', r.solusStartup.join(', '), 'ms');
console.log('Background CPU with 10 sites open (3 minutes, no input)');
r.chromeCpu = await idleCpu('chrome'); console.log('  Chrome', JSON.stringify(r.chromeCpu));
r.solusCpu = await idleCpu('solus'); console.log('  Solus ', JSON.stringify(r.solusCpu));
const md = `# Startup & CPU benchmark — ${r.when.slice(0, 10)}\n\n${r.machine}. Fresh profiles, default settings.\n\n| Measure | Chrome | Solus |\n|---|---|---|\n` +
  `| Startup to visible window, median of ${RUNS} (ms) | ${median(r.chromeStartup)} | ${median(r.solusStartup)} |\n` +
  `| Background CPU, 10 sites idle for 3 min (CPU-seconds) | ${(r.chromeCpu.cpuMs / 1000).toFixed(1)} | ${(r.solusCpu.cpuMs / 1000).toFixed(1)} |\n` +
  `| Same, as average % of the whole CPU | ${r.chromeCpu.avgPercentOfMachine}% | ${r.solusCpu.avgPercentOfMachine}% |\n`;
fs.mkdirSync('bench/results', { recursive: true });
const stamp = r.when.replace(/[:.]/g, '-').slice(0, 19);
fs.writeFileSync(`bench/results/cpu-startup-${stamp}.json`, JSON.stringify(r, null, 2));
fs.writeFileSync(`bench/results/cpu-startup-${stamp}.md`, md);
console.log('\n' + md);
