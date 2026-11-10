// First-run setup wizard. Every step can be skipped and changed later in Settings.
'use strict';
let S = {}, L = {};
let i = 0;
const set = (key, value) => call('setSetting', { key, value }).catch((e) => toast(e.message));
const setLocal = (key, value) => call('app.setLocal', { key, value }).catch((e) => toast(e.message));

function opts(cls, list, cur, onPick) {
  const el = h(`<div class="choice ${cls}">${list.map(([v, t, d]) => `<button class="opt${v === cur ? ' on' : ''}" data-v="${esc(v)}"><b>${esc(t)}</b>${d ? `<span>${d}</span>` : ''}</button>`).join('')}</div>`);
  el.onclick = (e) => { const b = e.target.closest('.opt'); if (!b) return; $$('.opt', el).forEach((x) => x.classList.toggle('on', x === b)); onPick(b.dataset.v); };
  return el;
}

const STEPS = [
  { id: 'hello', skip: false, render(box) {
    box.innerHTML = `<div class="hero"><img src="/img/logo-256.png" alt=""><h1>Welcome to Solus</h1><p class="muted" style="font-size:15px;max-width:520px;margin:0 auto">A fast, private browser built to use as little memory as possible. Let's set it up — it takes about a minute, and you can skip anything.</p></div>
      <div class="grid c3" style="margin-top:28px">${[['bolt', 'Light on memory', 'Unused tabs pause and unload automatically.'], ['shield', 'Private by default', 'Ads, trackers and third-party cookies blocked. No telemetry.'], ['sparkle', 'Built-in assistant', 'Summarise pages and organise tabs with your own AI key.']].map(([ic, t, d]) => `<div class="card pad">${icon(ic, 22)}<h3 style="margin-top:8px">${t}</h3><p class="muted small" style="margin:0">${d}</p></div>`).join('')}</div>`;
  } },
  { id: 'look', render(box) {
    box.innerHTML = '<h1>How should Solus look?</h1><p class="muted">Choose light or dark, or follow Windows.</p>';
    box.appendChild(opts('c3', [['system', 'Follow Windows', 'Switches automatically'], ['light', 'Light', ''], ['dark', 'Dark', '']], S.theme, (v) => { S.theme = v; set('theme', v); }));
    box.appendChild(h('<h3 style="margin-top:22px">Layout</h3>'));
    box.appendChild(opts('c3', [['compact', 'Compact', 'More room for pages'], ['comfortable', 'Comfortable', 'Balanced'], ['spacious', 'Spacious', 'Larger targets']], S.density, (v) => { S.density = v; set('density', v); }));
  } },
  { id: 'import', render(box) {
    box.innerHTML = '<h1>Bring your stuff from Chrome</h1><p class="muted">Solus copies your bookmarks, history and search shortcuts. Chrome itself isn\'t changed.</p><div id="imp" class="card pad"><span class="spinner"></span></div>';
    call('import.chromeProfiles').then((d) => {
      const imp = $('#imp', box);
      if (!d.profiles.length) { imp.innerHTML = '<p style="margin:0">Google Chrome wasn\'t found on this PC — nothing to import. You can import later from Settings.</p>'; return; }
      imp.innerHTML = `${d.running ? '<div class="banner warn">Chrome is open. For a complete history import, close Chrome first.</div>' : ''}
        <label>Chrome profile <select class="field" data-prof>${d.profiles.map((p) => `<option value="${esc(p.dir)}">${esc(p.name)}${p.email ? ' — ' + esc(p.email) : ''}</option>`).join('')}</select></label>
        ${[['bookmarks', 'Bookmarks'], ['history', 'History'], ['searchEngines', 'Search shortcuts'], ['settings', 'Start-up pages']].map(([k, l]) => `<label><input type="checkbox" data-w="${k}" checked> ${l}</label>`).join('')}
        <div style="margin-top:10px"><button class="btn primary" data-go>Import now</button> <span data-res class="small"></span></div>
        <p class="small muted" style="margin:14px 0 0">Passwords: Chrome only lets its own export read them. After setup, go to <b>Settings → Import from Chrome</b> for the two-click password import.</p>`;
      $('[data-go]', imp).onclick = () => {
        const what = {}; $$('[data-w]', imp).forEach((c) => (what[c.dataset.w] = c.checked));
        $('[data-go]', imp).disabled = true; $('[data-res]', imp).innerHTML = '<span class="spinner"></span> Importing…';
        call('import.chrome', { dir: $('[data-prof]', imp).value, what }).then((r) => {
          $('[data-res]', imp).innerHTML = `<span class="tick">${icon('check', 14)}</span> ${r.imported.bookmarks ?? 0} bookmarks and ${r.imported.history ?? 0} pages imported.` + (r.errors.length ? ` <span style="color:var(--danger)">${esc(r.errors[0])}</span>` : '');
        }).catch((e) => { $('[data-res]', imp).textContent = e.message; $('[data-go]', imp).disabled = false; });
      };
    }).catch(() => { $('#imp', box).textContent = 'Import is unavailable.'; });
  } },
  { id: 'search', render(box) {
    box.innerHTML = '<h1>Search engine</h1><p class="muted">Used when you type in the address bar.</p>';
    box.appendChild(opts('c2', (S.engines || []).map((e) => [e.id, e.name, e.id === 'google' ? 'Recommended' : '']), S.search_engine, (v) => { S.search_engine = v; set('search_engine', v); }));
  } },
  { id: 'privacy', render(box) {
    box.innerHTML = '<h1>Privacy</h1><p class="muted">Solus blocks ads and trackers while keeping sites working. You can change this per site from the shield button.</p>';
    box.appendChild(opts('c3', [['off', 'Off', 'No blocking'], ['balanced', 'Balanced', 'Blocks ads and trackers — recommended'], ['strict', 'Strict', 'Also cookie banners and social widgets; may break some sites']], S.blocking_level, (v) => { S.blocking_level = v; set('blocking_level', v); }));
    const c = h(`<div class="card pad" style="margin-top:16px">
      <label><input type="checkbox" data-k="block_third_party_cookies" ${S.block_third_party_cookies ? 'checked' : ''}> Block third-party cookies</label>
      <label><input type="checkbox" data-k="https_first" ${S.https_first ? 'checked' : ''}> Always use secure connections (HTTPS-First)</label>
      <label><input type="checkbox" data-k="search_suggestions" ${S.search_suggestions ? 'checked' : ''}> Show search suggestions as I type</label>
      <p class="small muted" style="margin:8px 0 0">Solus has no telemetry. Your history is stored encrypted on this PC.</p></div>`);
    c.onchange = (e) => { const k = e.target.dataset.k; if (k) set(k, e.target.checked); };
    box.appendChild(c);
  } },
  { id: 'performance', render(box) {
    box.innerHTML = '<h1>Memory &amp; performance</h1><p class="muted">How hard should Solus work to save memory and battery?</p>';
    box.appendChild(opts('c3', [['performance', 'Performance', 'Tabs stay ready the longest'], ['balanced', 'Balanced', 'Pause tabs after 5 min, unload after 45 — recommended'], ['efficiency', 'Efficiency', 'Lowest memory and battery use']], L.perf_profile, (v) => { L.perf_profile = v; setLocal('perf_profile', v); }));
    const c = h(`<div class="card pad" style="margin-top:16px"><label><input type="checkbox" ${L.auto_battery ? 'checked' : ''}> Switch to Efficiency automatically on battery</label></div>`);
    $('input', c).onchange = (e) => setLocal('auto_battery', e.target.checked);
    box.appendChild(c);
  } },
  { id: 'ai', render(box) {
    box.innerHTML = `<h1>AI assistant <span class="pill">optional</span></h1><p class="muted">Summarise and explain pages, compare tabs and tidy up your tabs. It uses your own API key, stored encrypted on this PC.</p>
      <div class="card pad"><label>Service <select class="field" data-p><option value="anthropic">Claude (Anthropic) — recommended</option><option value="groq">Groq — free tier</option></select></label>
      <label>API key <input class="field" type="password" data-k style="flex:1" placeholder="sk-ant-…" autocomplete="off"></label>
      <div class="small muted" data-help></div>
      <div style="margin-top:10px"><button class="btn primary" data-save>Save and test key</button> <span data-res class="small"></span></div>
      <p class="small muted" style="margin:12px 0 0">Claude API usage is billed by Anthropic (separate from a Claude subscription). The assistant can't see your history unless you allow it in Settings.</p></div>`;
    const p = $('[data-p]', box), k = $('[data-k]', box);
    const help = () => { $('[data-help]', box).innerHTML = p.value === 'groq' ? 'Get a free key at console.groq.com/keys' : 'Create a key at console.anthropic.com → Settings → API keys'; k.placeholder = p.value === 'groq' ? 'gsk_…' : 'sk-ant-…'; };
    p.onchange = help; help();
    $('[data-save]', box).onclick = async () => {
      $('[data-res]', box).innerHTML = '<span class="spinner"></span>';
      try {
        await call('setSetting', { key: 'ai.provider', value: p.value });
        await call('ai.setKey', { provider: p.value, key: k.value });
        await call('ai.testKey', { provider: p.value });
        $('[data-res]', box).innerHTML = `<span class="tick">${icon('check', 14)} Key works</span>`;
      } catch (e) { $('[data-res]', box).textContent = e.message; }
    };
  } },
  { id: 'sync', render(box) {
    box.innerHTML = `<h1>Sync &amp; backups <span class="pill">optional</span></h1>
      <p class="muted">Keep bookmarks, passwords, history, notes and settings in step across your PCs through your own cloud drive (OneDrive, Google Drive, Dropbox), encrypted with a passphrase only you know.</p>
      <div class="card pad"><p style="margin-top:0">Solus also makes an automatic local backup of your profile every day.</p>
      <button class="btn" data-go>Set up sync now</button> <span class="small muted">— or later from Settings → Sync &amp; backups</span></div>`;
    $('[data-go]', box).onclick = () => { finish().then(() => { location.href = 'solus://settings/#sync'; }); };
  } },
  { id: 'done', skip: false, render(box) {
    box.innerHTML = `<div class="hero">${icon('check', 56)}<h1>You're all set</h1><p class="muted">A few tips:</p></div>
      <div class="card pad" style="max-width:520px;margin:0 auto;line-height:1.9">
        <div><b>Ctrl+L</b> — address bar · <b>Ctrl+T</b> — new tab · <b>Ctrl+Shift+A</b> — search tabs</div>
        <div><b>Ctrl+Shift+E</b> — AI assistant · <b>Ctrl+Shift+S</b> — sidebar · <b>Ctrl+Shift+X</b> — split screen</div>
        <div>Use the <b>workspace</b> button at the left of the tabs to keep School, Personal and Research tabs apart.</div>
        <div>Closing the window keeps Solus in the notification area — choose <b>Exit Solus</b> in the menu to quit.</div>
        <div><button class="btn small" data-def>Make Solus my default browser</button></div></div>`;
    $('[data-def]', box).onclick = () => call('app.makeDefault');
  } },
];

function draw() {
  const st = STEPS[i];
  $('#steps').innerHTML = STEPS.map((_, j) => `<span class="${j <= i ? 'on' : ''}"></span>`).join('');
  const box = $('#step');
  box.innerHTML = '';
  st.render(box);
  $('#back').style.visibility = i === 0 ? 'hidden' : 'visible';
  $('#skip').hidden = st.skip === false || i === STEPS.length - 1;
  $('#next').textContent = i === 0 ? 'Get started' : i === STEPS.length - 1 ? 'Start browsing' : 'Next';
}
function finish() { return call('app.onboardingDone').catch(() => {}); }

$('#back').onclick = () => { if (i > 0) { i--; draw(); } };
$('#skip').onclick = () => { if (i < STEPS.length - 1) { i++; draw(); } };
$('#next').onclick = () => {
  if (i < STEPS.length - 1) { i++; draw(); return; }
  finish().then(() => { location.href = 'solus://newtab/'; });
};

(async () => {
  [S, L] = await Promise.all([call('getSettings'), call('app.local')]);
  draw();
})();
