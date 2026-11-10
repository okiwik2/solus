// Settings page. Every control writes a real setting through the host bridge.
'use strict';

let S = {};      // profile settings
let L = {};      // app-wide local state
let restartNeeded = false;

const SECTIONS = [
  ['appearance', 'palette', 'Appearance'], ['startup', 'home', 'Start-up & home page'], ['search', 'search', 'Search & address bar'],
  ['tabs', 'tabs', 'Tabs & workspaces'], ['privacy', 'shield', 'Privacy & security'], ['passwords', 'key', 'Passwords & autofill'],
  ['ai', 'sparkle', 'AI assistant'], ['sync', 'cloud', 'Sync & backups'], ['performance', 'bolt', 'Performance & memory'],
  ['downloads', 'download', 'Downloads'], ['languages', 'translate', 'Languages & translation'], ['shortcuts', 'grid', 'Keyboard shortcuts'],
  ['profiles', 'user', 'Profiles'], ['import', 'import', 'Import from Chrome'], ['system', 'tools', 'System'], ['about', 'info', 'About Solus'],
];

const set = (key, value) => call('setSetting', { key, value }).then((r) => { if (r && r.restartRequired) showRestart(); return r; }).catch((e) => toast(e.message));
const setLocal = (key, value) => call('app.setLocal', { key, value }).then((r) => { if (r && r.restartRequired) showRestart(); L = Object.assign(L, {}); return r; }).catch((e) => toast(e.message));
const get = (obj, path) => path.split('.').reduce((o, k) => (o ? o[k] : undefined), obj);

function showRestart() {
  if (restartNeeded) return;
  restartNeeded = true;
  const b = h(`<div class="banner warn" style="position:sticky;top:10px;z-index:5"><span style="flex:1">Some changes take effect after Solus restarts.</span><button class="btn small primary">Restart now</button></div>`);
  $('button', b).onclick = () => call('app.restart');
  $('#main').prepend(b);
}

// ---------------------------------------------------------------- building blocks
function item(label, desc, control) {
  const el = h(`<div class="item"><div class="grow"><div class="label">${esc(label)}</div>${desc ? `<div class="desc">${desc}</div>` : ''}</div></div>`);
  if (control) el.appendChild(typeof control === 'string' ? h(control) : control);
  return el;
}
function toggle(label, desc, key, opts = {}) {
  const src = opts.local ? L : S;
  const ctl = h(sw(!!get(src, key)));
  $('input', ctl).onchange = (e) => (opts.local ? setLocal(key, e.target.checked) : set(key, e.target.checked)).then(() => opts.after && opts.after(e.target.checked));
  return item(label, desc, ctl);
}
function seg(label, desc, key, options, opts = {}) {
  const src = opts.local ? L : S;
  const cur = get(src, key);
  const ctl = h(`<div class="seg">${options.map(([v, t]) => `<button data-v="${esc(v)}" class="${v === cur ? 'on' : ''}">${esc(t)}</button>`).join('')}</div>`);
  ctl.onclick = (e) => {
    const b = e.target.closest('button'); if (!b) return;
    $$('button', ctl).forEach((x) => x.classList.toggle('on', x === b));
    (opts.local ? setLocal(key, b.dataset.v) : set(key, b.dataset.v)).then(() => opts.after && opts.after(b.dataset.v));
  };
  return item(label, desc, ctl);
}
function select(label, desc, key, options, opts = {}) {
  const src = opts.local ? L : S;
  const cur = String(get(src, key));
  const ctl = h(`<select class="field">${options.map(([v, t]) => `<option value="${esc(v)}"${String(v) === cur ? ' selected' : ''}>${esc(t)}</option>`).join('')}</select>`);
  ctl.onchange = () => { const v = opts.number ? +ctl.value : ctl.value; (opts.local ? setLocal(key, v) : set(key, v)).then(() => opts.after && opts.after(v)); };
  return item(label, desc, ctl);
}
function card(...children) { const c = h('<div class="card"></div>'); children.flat().filter(Boolean).forEach((x) => c.appendChild(x)); return c; }
function section(id, title, intro) {
  const s = h(`<section class="sec" id="sec-${id}"><h2>${esc(title)}</h2>${intro ? `<p class="muted small" style="margin-top:-4px">${intro}</p>` : ''}</section>`);
  $('#main').appendChild(s);
  return s;
}
function listEditor(values, placeholder, onChange) {
  const box = h('<div style="padding:10px 18px 14px"></div>');
  const draw = () => {
    box.innerHTML = values.map((v, i) => `<div class="row-flex" style="margin-bottom:6px"><span style="flex:1;overflow:hidden;text-overflow:ellipsis">${esc(v)}</span><button class="icon-btn" data-rm="${i}" title="Remove">${icon('trash', 15)}</button></div>`).join('') +
      `<div class="row-flex"><input class="field" style="flex:1" placeholder="${esc(placeholder)}"><button class="btn small">Add</button></div>`;
    $('button.btn', box).onclick = () => { const v = $('input', box).value.trim(); if (v) { values.push(v); onChange(values); draw(); } };
    $('input', box).onkeydown = (e) => { if (e.key === 'Enter') $('button.btn', box).click(); };
    $$('[data-rm]', box).forEach((b) => (b.onclick = () => { values.splice(+b.dataset.rm, 1); onChange(values); draw(); }));
  };
  draw();
  return box;
}

// ---------------------------------------------------------------- sections
async function appearance() {
  const s = section('appearance', 'Appearance');
  const themes = (await call('themes.list')) || [];
  const dark = matchMedia('(prefers-color-scheme: dark)').matches;
  const sw2 = h(`<div class="theme-swatches">${themes.map((t) => `<button data-id="${esc(t.id)}" class="${t.id === S.theme_id ? 'on' : ''}"><span class="sw" style="background:linear-gradient(135deg, ${t[dark ? 'dark' : 'light'].frame} 50%, ${t.accent} 50%)"></span>${esc(t.name)}</button>`).join('')}</div>`);
  sw2.onclick = (e) => { const b = e.target.closest('button'); if (!b) return; $$('button', sw2).forEach((x) => x.classList.toggle('on', x === b)); set('theme_id', b.dataset.id); };
  const ed = h(`<button class="btn small">${icon('palette', 14)} Theme editor &amp; custom CSS</button>`);
  ed.onclick = () => call('openInternal', { page: 'themes' });
  s.appendChild(card(
    seg('Mode', 'System follows Windows\' light/dark setting automatically.', 'theme', [['system', 'System'], ['light', 'Light'], ['dark', 'Dark']]),
    item('Theme', '', h('<span></span>')),
    (() => { const d = h('<div style="padding:0 18px 14px"></div>'); d.appendChild(sw2); d.appendChild(h('<div style="height:10px"></div>')); d.appendChild(ed); return d; })(),
    seg('Interface density', 'How much space the tab strip, toolbar and bookmarks bar use.', 'density', [['compact', 'Compact'], ['comfortable', 'Comfortable'], ['spacious', 'Spacious']]),
    toggle('Subtle animations', 'Spinners, transitions and hover effects.', 'animations'),
    toggle('Show home button', '', 'show_home_button'),
    toggle('Tab hover cards', 'Show a preview card when you point at a tab.', 'tab_hover_cards'),
  ));
}

function startup() {
  const s = section('startup', 'Start-up & home page');
  const urls = [...(S.startup_urls || [])];
  const ed = listEditor(urls, 'https://example.com', (v) => set('startup_urls', v));
  ed.hidden = S.startup !== 'urls';
  s.appendChild(card(
    seg('When Solus starts', '', 'startup', [['restore', 'Continue where you left off'], ['newtab', 'Open the new tab page'], ['urls', 'Open specific pages']], { after: (v) => (ed.hidden = v !== 'urls') }),
    ed,
    (() => {
      const i = h(`<input class="field" style="width:280px" placeholder="New tab page" value="${esc(S.homepage || '')}">`);
      i.onchange = () => { const v = i.value.trim(); if (v && !/^https?:\/\//.test(v)) { toast('Use a full address starting with https://'); return; } set('homepage', v); };
      return item('Home page', 'Leave empty to use the new tab page.', i);
    })(),
  ));
}

function search() {
  const s = section('search', 'Search & address bar');
  const shortcuts = [...(S.search_shortcuts || [])];
  const table = h('<div style="padding:4px 18px 14px"></div>');
  const draw = () => {
    table.innerHTML = `<table class="list"><tr><th>Shortcut</th><th>Name</th><th>URL (%s = search terms)</th><th></th></tr>${shortcuts.map((x, i) => `<tr><td><code>${esc(x.keyword)}</code></td><td>${esc(x.name)}</td><td class="small" style="max-width:300px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(x.url)}</td><td><button class="icon-btn" data-rm="${i}">${icon('trash', 14)}</button></td></tr>`).join('')}</table>
      <div class="row-flex" style="margin-top:8px"><input class="field" data-k placeholder="yt" style="width:80px"><input class="field" data-n placeholder="Name" style="width:140px"><input class="field" data-u placeholder="https://www.youtube.com/results?search_query=%s" style="flex:1"><button class="btn small">Add</button></div>`;
    $$('[data-rm]', table).forEach((b) => (b.onclick = () => { shortcuts.splice(+b.dataset.rm, 1); set('search_shortcuts', shortcuts); draw(); }));
    $('button.btn', table).onclick = () => {
      const k = $('[data-k]', table).value.trim(), n = $('[data-n]', table).value.trim(), u = $('[data-u]', table).value.trim();
      if (!k || !/^https?:\/\//.test(u) || !u.includes('%s')) { toast('Give a shortcut word and an https:// URL containing %s'); return; }
      shortcuts.push({ keyword: k, name: n || k, url: u }); set('search_shortcuts', shortcuts); draw();
    };
  };
  draw();
  s.appendChild(card(
    select('Search engine', 'Used in the address bar and on the new tab page.', 'search_engine', (S.engines || []).map((e) => [e.id, e.name])),
    toggle('Show search suggestions', 'Sends what you type in the address bar to your search engine to suggest searches.', 'search_suggestions'),
    item('Search shortcuts', 'Type a shortcut, a space and your search — e.g. <code>yt cats</code>.', null), table,
  ));
}

async function tabs() {
  const s = section('tabs', 'Tabs & workspaces');
  s.appendChild(card(
    toggle('Group tabs automatically', 'When several tabs from the same site are open, put them in a group.', 'auto_group_tabs'),
    select('Group after', '', 'auto_group_threshold', [[3, '3 tabs from a site'], [4, '4 tabs from a site'], [5, '5 tabs from a site'], [6, '6 tabs from a site']], { number: true }),
    toggle('Detect duplicate tabs', 'Offer to switch when a page is already open in another tab.', 'duplicate_detection'),
  ));
  const w = h('<section class="sec" id="sec-workspaces"><h2>Workspaces</h2><p class="muted small" style="margin-top:-4px">Each workspace keeps its own set of tabs — for example School, Personal and Research. Switch with the button at the left of the tab strip or Ctrl+Shift+1–9.</p></section>');
  $('#main').appendChild(w);
  const box = card();
  w.appendChild(box);
  const draw = async () => {
    const list = await call('workspaces.list');
    box.innerHTML = '';
    list.forEach((x) => {
      const row = item('', '', null);
      row.innerHTML = `<span style="width:12px;height:12px;border-radius:4px;background:${esc(x.color || 'var(--accent)')}"></span><input class="field" value="${esc(x.name)}" style="flex:1"><input type="color" value="${/^#[0-9a-f]{6}$/i.test(x.color) ? x.color : '#e8590c'}" title="Colour">${x.id ? `<button class="icon-btn" data-del title="Delete (its tabs move to ${esc(list[0].name)})">${icon('trash', 15)}</button>` : '<span class="muted small" style="width:32px"></span>'}`;
      $('input.field', row).onchange = (e) => call('workspaces.update', { id: x.id, name: e.target.value });
      $('input[type=color]', row).onchange = (e) => call('workspaces.update', { id: x.id, color: e.target.value }).then(draw);
      const del = $('[data-del]', row);
      if (del) del.onclick = () => confirmBox('Delete workspace?', `Its tabs move to "${list[0].name}".`, 'Delete', true).then((ok) => ok && call('workspaces.delete', { id: x.id }).then(draw));
      box.appendChild(row);
    });
    const add = h(`<div class="item"><input class="field" placeholder="New workspace name (e.g. School)" style="flex:1"><button class="btn small">Add workspace</button></div>`);
    $('button', add).onclick = () => { const n = $('input', add).value.trim(); if (n) call('workspaces.create', { name: n, color: '#1c7ed6' }).then(draw).catch((e) => toast(e.message)); };
    box.appendChild(add);
    const tmpl = h(`<div class="item"><span class="muted small" style="flex:1">Quick add:</span>${['School', 'Research', 'Work', 'Shopping'].map((n) => `<button class="btn small" data-n="${n}">${n}</button>`).join('')}</div>`);
    tmpl.onclick = (e) => { const b = e.target.closest('[data-n]'); if (b && !list.some((x) => x.name === b.dataset.n)) call('workspaces.create', { name: b.dataset.n, color: { School: '#1c7ed6', Research: '#2f9e44', Work: '#7048e8', Shopping: '#d6336c' }[b.dataset.n] }).then(draw); };
    box.appendChild(tmpl);
  };
  draw();
}

async function privacy() {
  const s = section('privacy', 'Privacy & security', 'Solus has no telemetry: it does not collect or send usage data. See the <a href="solus://privacy/">privacy dashboard</a> for what is protected right now.');
  const ex = [...(S.site_exceptions || [])];
  s.appendChild(card(
    seg('Ad & tracker blocking', 'Balanced uses EasyList + EasyPrivacy and avoids most breakage. Strict adds uBlock Privacy and annoyance filters.', 'blocking_level', [['off', 'Off'], ['balanced', 'Balanced'], ['strict', 'Strict']]),
    select('Tracking prevention', 'The engine\'s built-in protection against known trackers.', 'tracking_prevention', [['basic', 'Basic'], ['balanced', 'Balanced (recommended)'], ['strict', 'Strict']]),
    toggle('Block third-party cookies', 'Stops sites you\'re not visiting from tracking you with cookies. Applies after a restart.', 'block_third_party_cookies'),
    toggle('Always use secure connections (HTTPS-First)', 'Upgrades web addresses to HTTPS and warns when a site doesn\'t support it.', 'https_first'),
    toggle('Microsoft Defender SmartScreen', 'Warns about dangerous sites and downloads. (Google Safe Browsing is not available to Chromium-based apps; SmartScreen provides the equivalent protection.)', 'smartscreen'),
    toggle('Send a "Do Not Track" request', 'Most sites ignore it. Applies after a restart.', 'do_not_track'),
    item('Sites where blocking is off', 'Turn protection off for a site from the shield button in the toolbar.', null),
    listEditor(ex, 'example.com', (v) => set('site_exceptions', v)),
  ));
  // filter lists
  const lists = await call('privacy.lists').catch(() => null);
  if (lists) {
    const lc = card(...lists.lists.map((l) => item(l.name.replace('.txt', ''), `${l.active ? '<span class="pill ok">in use</span> ' : ''}${fmtBytes(l.size)} · updated ${l.modified ? timeAgo(l.modified) : 'never'}${l.updated ? '' : ' (built-in copy)'}`, null)));
    const up = h('<div class="item"><span class="grow muted small">Lists update automatically every few days.</span><button class="btn small">Update now</button></div>');
    $('button', up).onclick = () => call('privacy.updateLists').then(() => toast('Updating filter lists in the background…'));
    lc.appendChild(up);
    s.appendChild(h('<h3 style="margin:18px 0 8px">Filter lists</h3>'));
    s.appendChild(lc);
  }
  // site permissions
  const ps = h('<section class="sec" id="sec-site"><h2>Site permissions</h2><p class="muted small" style="margin-top:-4px">Sites ask before using your camera, microphone, location or notifications. Your answers are listed here.</p></section>');
  $('#main').appendChild(ps);
  const pc = card();
  ps.appendChild(pc);
  const drawPerms = async () => {
    const rows = await call('permissions.list');
    pc.innerHTML = rows.length ? `<div style="padding:4px 18px 10px"><table class="list perm-grid"><tr><th>Site</th><th>Permission</th><th>Setting</th><th></th></tr>${rows.map((r, i) => `<tr><td>${esc(r.origin)}</td><td>${esc(r.label)}</td><td><select class="field" data-i="${i}"><option value="allow"${r.state === 'allow' ? ' selected' : ''}>Allow</option><option value="block"${r.state === 'block' ? ' selected' : ''}>Block</option><option value="default">Ask</option></select></td><td><button class="icon-btn" data-reset="${esc(r.origin)}" title="Reset all for this site">${icon('reload', 14)}</button></td></tr>`).join('')}</table></div>` : '<div class="item muted">No site has asked for a permission yet.</div>';
    $$('select', pc).forEach((sel) => (sel.onchange = () => { const r = rows[+sel.dataset.i]; call('permissions.set', { origin: r.origin, kind: r.kind, state: sel.value }).then(drawPerms); }));
    $$('[data-reset]', pc).forEach((b) => (b.onclick = () => call('permissions.reset', { origin: b.dataset.reset }).then(drawPerms)));
  };
  drawPerms();
  // clear data
  const cs = h('<section class="sec" id="sec-clear"><h2>Clear browsing data</h2><p class="muted small" style="margin-top:-4px">Solus never deletes your data automatically. Choose what to remove:</p></section>');
  $('#main').appendChild(cs);
  const types = [['history', 'Browsing history', true], ['downloads', 'Download history', true], ['cookies', 'Cookies and site data (signs you out of most sites)', false], ['cache', 'Cached images and files', true], ['autofill', 'Autofill form data', false], ['site_settings', 'Site permissions', false], ['passwords', 'Saved passwords', false]];
  const cc = card(h(`<div style="padding:12px 18px">${types.map(([k, l, d]) => `<label class="row-flex" style="padding:4px 0"><input type="checkbox" data-t="${k}" ${d ? 'checked' : ''}> ${esc(l)}</label>`).join('')}
    <div class="row-flex" style="margin-top:12px"><select class="field" data-range><option value="hour">Last hour</option><option value="day">Last 24 hours</option><option value="week">Last 7 days</option><option value="month">Last 4 weeks</option><option value="all" selected>All time</option></select><span style="flex:1"></span><button class="btn danger" data-go>Clear data</button></div></div>`));
  $('[data-go]', cc).onclick = async () => {
    const t = $$('[data-t]', cc).filter((x) => x.checked).map((x) => x.dataset.t);
    if (!t.length) return;
    if (!(await confirmBox('Clear browsing data?', 'This can\'t be undone (Solus makes a local backup first).', 'Clear', true))) return;
    await call('backup.create').catch(() => {});
    call('app.clearData', { types: t, range: $('[data-range]', cc).value }).then(() => toast('Cleared.')).catch((e) => toast(e.message));
  };
  cs.appendChild(cc);
}

function passwords() {
  const s = section('passwords', 'Passwords & autofill');
  const open = h(`<button class="btn small">Open passwords</button>`);
  open.onclick = () => call('openInternal', { page: 'passwords' });
  s.appendChild(card(
    toggle('Offer to save passwords', 'Saved passwords are encrypted on this PC. Viewing or exporting them asks Windows to confirm it\'s you.', 'password_manager'),
    toggle('Autofill addresses and forms', 'The engine remembers what you type into address and contact forms.', 'autofill_forms'),
    item('Saved passwords', 'View, edit, import from Chrome or export.', open),
  ));
}

async function ai() {
  const s = section('ai', 'AI assistant', 'The assistant runs on your own API key. Each request is sent to the service you choose; nothing is sent until you ask it something.');
  const st = await call('ai.status').catch(() => null);
  if (!st) return;
  const prov = st.provider;
  const models = (st.models[prov] || []).map((m) => [m.id, `${m.name} — $${m.in}/$${m.out} per million tokens`]);
  const keyRow = (p, name, url) => {
    const has = p === 'groq' ? st.groqKey : st.anthropicKey;
    const el = h(`<div class="item"><div class="grow"><div class="label">${name} API key</div><div class="desc">${has ? '<span class="pill ok">saved, encrypted</span>' : 'Not set'} — get one at <a href="${url}">${url.replace('https://', '')}</a></div></div><input class="field" type="password" placeholder="${has ? 'Replace key…' : 'Paste key'}" style="width:230px" autocomplete="off"><button class="btn small">Save</button>${has ? '<button class="btn small" data-test>Test</button><button class="btn small danger" data-rm>Remove</button>' : ''}</div>`);
    $('button.btn', el).onclick = () => call('ai.setKey', { provider: p, key: $('input', el).value }).then(() => call('ai.testKey', { provider: p })).then(() => { toast('Key saved and verified.'); rerender(); }).catch((e) => toast(e.message));
    const t = $('[data-test]', el); if (t) t.onclick = () => call('ai.testKey', { provider: p }).then(() => toast('The key works.')).catch((e) => toast(e.message));
    const r = $('[data-rm]', el); if (r) r.onclick = () => call('ai.setKey', { provider: p, key: '' }).then(rerender);
    return el;
  };
  s.appendChild(card(
    toggle('Enable the assistant', '', 'ai.enabled'),
    select('Service', 'Claude gives the best results for reading pages and taking actions. Groq has a free tier.', 'ai.provider', [['anthropic', 'Claude (Anthropic)'], ['groq', 'Groq']], { after: () => set('ai.model', '').then(rerender) }),
    select('Model', 'Prices are the provider\'s list prices; a typical page summary uses 5–15 thousand tokens.', 'ai.model', [['', 'Default (' + (prov === 'groq' ? 'GPT-OSS 120B' : 'Claude Opus 5.5') + ')'], ...models]),
    keyRow('anthropic', 'Anthropic', 'https://console.anthropic.com/settings/keys'),
    keyRow('groq', 'Groq', 'https://console.groq.com/keys'),
  ));
  s.appendChild(h('<h3 style="margin:18px 0 8px">What the assistant may do</h3>'));
  s.appendChild(card(
    toggle('Read the page you\'re on', 'Sends the current page\'s text with your question.', 'ai.permissions.read_page'),
    toggle('See and read your open tabs', 'Needed to compare tabs or organise them.', 'ai.permissions.read_tabs'),
    toggle('Search your browsing history', '<b>Off by default.</b> Only turn on if you want the assistant to find pages you visited.', 'ai.permissions.read_history'),
    toggle('Open, switch, group and close tabs', '', 'ai.permissions.manage_tabs'),
    toggle('Manage bookmarks', '', 'ai.permissions.manage_bookmarks'),
    toggle('Manage workspaces', '', 'ai.permissions.manage_workspaces'),
    toggle('Ask before closing tabs or deleting bookmarks', '', 'ai.confirm_actions'),
    toggle('Allow in Incognito windows', 'Off keeps private browsing private.', 'ai.permissions.in_incognito'),
  ));
}
function rerender() { load(location.hash); }

async function sync() {
  const s = section('sync', 'Sync & backups', 'Sync keeps bookmarks, passwords, history, notes, tasks, workspaces and settings the same on your PCs, using a folder your cloud drive already syncs (OneDrive, Google Drive, Dropbox). Everything is encrypted with your passphrase before it leaves this PC — neither Solus nor your cloud provider can read it.');
  const st = await call('sync.status').catch(() => null);
  if (!st) return;
  if (st.sync.enabled) {
    const now = h('<button class="btn small primary">Sync now</button>');
    now.onclick = () => call('sync.now').then((r) => toast(`Synced (${r.merged} changes from ${r.devices.length} other device${r.devices.length === 1 ? '' : 's'}).`)).catch((e) => toast(e.message));
    const off = h('<button class="btn small danger">Turn off sync</button>');
    off.onclick = () => confirmBox('Turn off sync?', 'Your data stays on this PC and in the sync folder.', 'Turn off', true).then((ok) => ok && call('sync.disable').then(rerender));
    s.appendChild(card(
      item('Sync folder', `${esc(st.sync.folder)}<br>Last sync: ${timeAgo(st.sync.last_sync)}${st.sync.last_error ? ` — <span style="color:var(--danger)">${esc(st.sync.last_error)}</span>` : ''}`, now),
      toggle('Bookmarks', '', 'sync.bookmarks'), toggle('Passwords', '', 'sync.passwords'), toggle('History', 'Your most recent 5,000 pages.', 'sync.history'),
      toggle('Notes', '', 'sync.notes'), toggle('Tasks & reminders', '', 'sync.tasks'), toggle('Workspaces', '', 'sync.workspaces'), toggle('Settings & themes', '', 'sync.settings'),
      item('This PC\'s name', 'Shown on your other devices.', (() => { const i = h(`<input class="field" value="${esc(st.deviceName)}" style="width:200px">`); i.onchange = () => setLocal('device_name', i.value.trim() || 'This PC'); return i; })()),
      item('', '', off),
    ));
  } else {
    const box = h(`<div style="padding:16px 18px">
      <p class="small" style="margin-top:0"><b>1.</b> Choose a folder inside your cloud drive.</p>
      <div class="row-flex">${(st.suggested || []).map((f) => `<button class="btn small" data-f="${esc(f.path)}">${icon('cloud', 14)} ${esc(f.name)}</button>`).join('')}<button class="btn small" data-browse>Choose folder…</button></div>
      <div class="small muted" data-folder style="margin:8px 0 14px">No folder chosen</div>
      <p class="small"><b>2.</b> Choose a passphrase (at least 10 characters). Use the same passphrase on each PC. <b>If you lose it, synced data can't be recovered.</b></p>
      <div class="row-flex"><input class="field" type="password" data-p1 placeholder="Passphrase" style="width:220px"><input class="field" type="password" data-p2 placeholder="Repeat passphrase" style="width:220px"></div>
      <div style="margin-top:14px"><button class="btn primary" data-go>Turn on sync</button></div></div>`);
    let folder = '';
    const pick = (f) => { folder = f; $('[data-folder]', box).textContent = f + '\\Solus Sync'; };
    $$('[data-f]', box).forEach((b) => (b.onclick = () => pick(b.dataset.f)));
    $('[data-browse]', box).onclick = () => call('app.pickFolder', { title: 'Choose a folder in your cloud drive' }).then((p) => p && pick(p));
    $('[data-go]', box).onclick = () => {
      const p1 = $('[data-p1]', box).value, p2 = $('[data-p2]', box).value;
      if (!folder) { toast('Choose a folder first'); return; }
      if (p1 !== p2) { toast('The passphrases don\'t match'); return; }
      $('[data-go]', box).disabled = true; $('[data-go]', box).innerHTML = '<span class="spinner"></span> Securing…';
      call('sync.setup', { folder, passphrase: p1 }).then((r) => { toast(r.created ? 'Sync is on. Use the same folder and passphrase on your other PCs.' : 'Joined your existing sync.'); rerender(); })
        .catch((e) => { toast(e.message); $('[data-go]', box).disabled = false; $('[data-go]', box).textContent = 'Turn on sync'; });
    };
    s.appendChild(card(box));
  }
  // backups
  const b = h('<section class="sec" id="sec-backups"><h3 style="margin:18px 0 8px">Backups</h3></section>');
  s.appendChild(b);
  const lb = await call('backup.list').catch(() => []);
  const mk = h('<button class="btn small">Back up now</button>');
  mk.onclick = () => call('backup.create').then(() => { toast('Backup created.'); rerender(); }).catch((e) => toast(e.message));
  const of = h('<button class="btn small">Open folder</button>');
  of.onclick = () => call('backup.openFolder');
  const list = h(`<div style="padding:4px 18px 12px">${lb.slice(0, 8).map((x) => `<div class="row-flex" style="padding:4px 0"><span style="flex:1">${new Date(x.created).toLocaleString()} <span class="muted small">· ${esc(x.reason || '')} · ${fmtBytes(x.size)}</span></span><button class="btn small" data-restore="${esc(x.name)}">Restore</button></div>`).join('') || '<span class="muted small">No local backups yet.</span>'}</div>`);
  $$('[data-restore]', list).forEach((x) => (x.onclick = () => confirmBox('Restore this backup?', 'Solus will restart and your bookmarks, history, passwords and settings return to this point. (Your current data is kept aside.)', 'Restore and restart', true).then((ok) => ok && call('backup.restore', { name: x.dataset.restore }).then(() => call('app.restart')))));
  b.appendChild(card(
    toggle('Automatic local backups', 'A daily snapshot of this profile, kept on this PC.', 'local_backups'),
    select('Keep', '', 'backup_keep', [[3, '3 backups'], [7, '7 backups'], [14, '14 backups'], [30, '30 backups']], { number: true }),
    st.sync.enabled ? toggle('Weekly encrypted cloud backups', 'Saved in the sync folder; restorable on a new PC with your passphrase.', 'sync.cloud_backups') : null,
    item('Local backups', '', (() => { const d = h('<div class="row-flex"></div>'); d.append(mk, of); return d; })()), list,
  ));
  if (st.sync.enabled) {
    const cb = await call('sync.listBackups').catch(() => []);
    const now2 = h('<button class="btn small">Cloud backup now</button>');
    now2.onclick = () => call('sync.backupNow').then(() => { toast('Encrypted cloud backup saved.'); rerender(); }).catch((e) => toast(e.message));
    const cl = h(`<div style="padding:4px 18px 12px">${cb.slice(0, 6).map((x) => `<div class="row-flex" style="padding:4px 0"><span style="flex:1">${esc(x.name)} <span class="muted small">· ${fmtBytes(x.size)}</span></span><button class="btn small" data-cr="${esc(x.name)}">Restore</button></div>`).join('') || '<span class="muted small">No cloud backups yet.</span>'}</div>`);
    $$('[data-cr]', cl).forEach((x) => (x.onclick = () => confirmBox('Restore from cloud backup?', 'Its bookmarks, passwords, notes and settings are merged into this profile (newest version of each item wins).', 'Restore').then((ok) => ok && call('sync.restoreBackup', { name: x.dataset.cr }).then((r) => toast(`Restored ${r.restored} items.`)).catch((e) => toast(e.message)))));
    b.appendChild(card(item('Cloud backups', 'Encrypted with your sync passphrase.', now2), cl));
  }
}

async function performance() {
  const s = section('performance', 'Performance & memory', 'Solus saves memory by pausing tabs you aren\'t using and, later, unloading them completely (they reload when you click them). The <a href="solus://taskmanager/">task manager</a> shows what each tab uses.');
  const st = await call('perf.status').catch(() => ({}));
  const desc = { performance: 'Keeps tabs ready longest. Uses the most memory.', balanced: 'Pauses tabs after 5 minutes, unloads after 45. Recommended.', efficiency: 'Pauses tabs after 1 minute, unloads after 10. Lowest memory and battery use.' };
  const d = h(`<div class="desc" style="padding:0 18px 12px;margin-top:-6px">${esc(desc[L.perf_profile] || '')}${st.battery || st.energySaver ? ' <span class="pill warn">On battery: Efficiency is active</span>' : ''}</div>`);
  const rules = L.rules || {};
  const custom = h(`<div style="padding:4px 18px 14px" ${L.custom_rules ? '' : 'hidden'}><div class="row-flex"><span style="flex:1">Pause inactive tabs after</span><input class="field" type="number" min="1" max="600" value="${rules.freeze_after_min}" data-f style="width:80px"> min</div>
    <div class="row-flex" style="margin-top:6px"><span style="flex:1">Unload inactive tabs after</span><input class="field" type="number" min="0" max="1440" value="${rules.discard_after_min}" data-d style="width:80px"> min <span class="muted small">(0 = only when memory is low)</span></div></div>`);
  const saveRules = () => setLocal('rules', Object.assign({}, rules, { freeze_after_min: +$('[data-f]', custom).value || 5, discard_after_min: +$('[data-d]', custom).value || 0 }));
  $('[data-f]', custom).onchange = saveRules; $('[data-d]', custom).onchange = saveRules;
  const ex = [...(rules.exceptions || [])];
  s.appendChild(card(
    seg('Performance profile', '', 'perf_profile', [['performance', 'Performance'], ['balanced', 'Balanced'], ['efficiency', 'Efficiency']], { local: true, after: (v) => (d.textContent = desc[v]) }), d,
    toggle('Use Efficiency automatically on battery', 'Also when Windows energy saver is on.', 'auto_battery', { local: true }),
    toggle('Custom timings', '', 'custom_rules', { local: true, after: (v) => (custom.hidden = !v) }), custom,
    toggle('Never pause tabs playing audio', '', 'rules.never_audio', { local: true }),
    toggle('Never unload pinned tabs', '', 'rules.never_pinned', { local: true }),
    toggle('Never unload pages with unsaved form input', '', 'rules.never_forms', { local: true }),
    item('Always keep these sites active', 'For example a chat or music site.', null),
    listEditor(ex, 'web.whatsapp.com', (v) => setLocal('rules', Object.assign({}, L.rules, { exceptions: v }))),
    toggle('Preload pages you\'re likely to open', 'Only from a new tab, for sites you visit often, and only when plenty of memory is free.', 'preload', { local: true }),
    toggle('Use hardware acceleration', 'Uses the graphics card for smoother scrolling and video. Applies after a restart.', 'gpu_acceleration', { local: true }),
  ));
}

function downloads() {
  const s = section('downloads', 'Downloads');
  const loc = h(`<div class="row-flex"><span class="muted small" data-p>${esc(S.download_dir || 'Downloads folder')}</span><button class="btn small">Change…</button></div>`);
  $('button', loc).onclick = () => call('app.pickFolder', { title: 'Save downloads to' }).then((p) => { if (p) { set('download_dir', p); $('[data-p]', loc).textContent = p; } });
  s.appendChild(card(item('Save files to', '', loc), toggle('Ask where to save each file', '', 'download_ask')));
}

async function languages() {
  const s = section('languages', 'Languages & translation', 'Translation uses Google Translate\'s web service, so it works for public pages (not pages that need you to be signed in).');
  const l = await call('translate.languages').catch(() => ({ languages: [], system: 'en' }));
  const names = Object.fromEntries(l.languages.map((x) => [x.code, x.name]));
  s.appendChild(card(
    toggle('Offer to translate pages in other languages', '', 'translate_offer'),
    select('Translate into', '', 'translate_target', [['', `Windows language (${names[l.system] || l.system})`], ...l.languages.map((x) => [x.code, x.name])]),
    item('Always translate', (S.translate_always || []).map((c) => names[c] || c).join(', ') || 'None', null),
    item('Never translate', (S.translate_never || []).map((c) => names[c] || c).join(', ') || 'None', (() => { const b = h('<button class="btn small">Reset both lists</button>'); b.onclick = () => set('translate_always', []).then(() => set('translate_never', [])).then(rerender); return b; })()),
  ));
}

function shortcuts() {
  const s = section('shortcuts', 'Keyboard shortcuts', 'Click a shortcut, then press the new key combination. Press Backspace to remove it, Esc to cancel.');
  const box = card();
  const list = window.__shortcuts || [];
  const fmt = (e) => {
    const k = e.key.length === 1 ? e.key.toUpperCase() : ({ ArrowLeft: 'Left', ArrowRight: 'Right', ArrowUp: 'Up', ArrowDown: 'Down', ' ': 'Space', Escape: 'Esc', Delete: 'Delete', Home: 'Home', End: 'End', Tab: 'Tab' }[e.key] || (/^F\d+$/.test(e.key) ? e.key : null));
    if (!k || ['Control', 'Shift', 'Alt'].includes(e.key)) return null;
    return (e.ctrlKey ? 'Ctrl+' : '') + (e.shiftKey ? 'Shift+' : '') + (e.altKey ? 'Alt+' : '') + k;
  };
  for (const sc of list) {
    const inp = h(`<input class="field kbd-input" readonly value="${esc(sc.keys)}">`);
    inp.onfocus = () => inp.classList.add('rec');
    inp.onblur = () => { inp.classList.remove('rec'); inp.value = sc.keys; };
    inp.onkeydown = (e) => {
      e.preventDefault();
      if (e.key === 'Escape') { inp.blur(); return; }
      if (e.key === 'Backspace') { sc.keys = ''; save(sc); inp.blur(); return; }
      const k = fmt(e); if (!k) return;
      if (!/^(Ctrl|Alt|F\d)/.test(k)) { toast('Use Ctrl or Alt with a key, or a function key'); return; }
      const clash = list.find((x) => x !== sc && x.keys === k);
      if (clash) { toast(`${k} is already used for "${clash.label}"`); return; }
      sc.keys = k; save(sc); inp.blur();
    };
    box.appendChild(item(sc.label, sc.keys !== sc.default ? `Default: ${esc(sc.default)}` : '', inp));
  }
  const reset = h('<div class="item"><span class="grow"></span><button class="btn small">Reset all to defaults</button></div>');
  $('button', reset).onclick = () => set('shortcuts', {}).then(rerender);
  box.appendChild(reset);
  s.appendChild(box);
  function save() {
    const map = {};
    list.forEach((x) => { if (x.keys !== x.default) map[x.action] = x.keys; });
    set('shortcuts', map);
  }
}

async function profiles() {
  const s = section('profiles', 'Profiles', 'Profiles keep separate bookmarks, history, passwords, cookies, extensions and settings — for example Personal and Work.');
  const d = await call('profiles.list');
  const box = card();
  for (const p of d.profiles) {
    const row = h(`<div class="item"><span style="width:28px;height:28px;border-radius:50%;background:${esc(p.color)};color:#fff;display:grid;place-items:center;font-weight:600">${esc(p.name.slice(0, 1).toUpperCase())}</span><input class="field" value="${esc(p.name)}" style="flex:1"><input type="color" value="${esc(p.color)}">${p.id === d.current ? '<span class="pill">This window</span>' : '<button class="btn small" data-open>Open</button>'}${p.id !== 'default' ? '<button class="icon-btn" data-del>' + icon('trash', 15) + '</button>' : ''}</div>`);
    $('input.field', row).onchange = (e) => call('profiles.rename', { id: p.id, name: e.target.value });
    $('input[type=color]', row).onchange = (e) => call('profiles.rename', { id: p.id, name: $('input.field', row).value, color: e.target.value });
    const o = $('[data-open]', row); if (o) o.onclick = () => call('profiles.open', { id: p.id });
    const del = $('[data-del]', row); if (del) del.onclick = () => confirmBox(`Delete profile "${p.name}"?`, 'Its data is moved to a "Deleted Profiles" folder.', 'Delete', true).then((ok) => ok && call('profiles.delete', { id: p.id }).then(rerender).catch((e) => toast(e.message)));
    box.appendChild(row);
  }
  const add = h('<div class="item"><input class="field" placeholder="New profile name" style="flex:1"><button class="btn small">Add profile</button></div>');
  $('button', add).onclick = () => { const n = $('input', add).value.trim(); if (n) call('profiles.create', { name: n }).then(rerender).catch((e) => toast(e.message)); };
  box.appendChild(add);
  s.appendChild(box);
}

async function importSec() {
  const s = section('import', 'Import from Chrome', 'Solus reads a copy of Chrome\'s data and never changes Chrome\'s files. A backup of your Solus profile is made before importing.');
  const d = await call('import.chromeProfiles').catch(() => null);
  if (!d || !d.installed && !d.profiles.length) { s.appendChild(card(item('Google Chrome was not found on this PC', '', null))); return; }
  const box = h(`<div style="padding:14px 18px">
    ${d.running ? '<div class="banner warn">Chrome is open. Bookmarks import fine, but for a complete history import, close Chrome first.</div>' : ''}
    <div class="row-flex"><span>Chrome profile</span><select class="field" data-prof>${d.profiles.map((p) => `<option value="${esc(p.dir)}">${esc(p.name)}${p.email ? ' — ' + esc(p.email) : ''}</option>`).join('')}</select></div>
    <div style="margin:12px 0">${[['bookmarks', 'Bookmarks'], ['history', 'History'], ['searchEngines', 'Search shortcuts'], ['settings', 'Start-up pages and home page']].map(([k, l]) => `<label class="row-flex" style="padding:3px 0"><input type="checkbox" data-w="${k}" checked> ${l}</label>`).join('')}</div>
    <button class="btn primary" data-go>Import</button> <span data-res class="small"></span></div>`);
  $('[data-go]', box).onclick = () => {
    const what = {}; $$('[data-w]', box).forEach((c) => (what[c.dataset.w] = c.checked));
    $('[data-go]', box).disabled = true; $('[data-res]', box).innerHTML = '<span class="spinner"></span> Importing…';
    call('import.chrome', { dir: $('[data-prof]', box).value, what }).then((r) => {
      const im = r.imported;
      $('[data-res]', box).innerHTML = `Imported ${im.bookmarks ?? 0} bookmarks, ${im.history ?? 0} history entries${im.searchShortcuts ? `, ${im.searchShortcuts} search shortcuts` : ''}.` + (r.errors.length ? `<br><span style="color:var(--danger)">${esc(r.errors.join('; '))}</span>` : '');
    }).catch((e) => { $('[data-res]', box).textContent = e.message; }).finally(() => ($('[data-go]', box).disabled = false));
  };
  s.appendChild(card(box));
  const pw = h(`<div style="padding:14px 18px"><p style="margin-top:0"><b>Passwords</b> — Chrome encrypts saved passwords so that only Chrome can read them, so they come across through Chrome's own export:</p>
    <ol class="small" style="line-height:1.8"><li><button class="btn small" data-chrome>Open Chrome's password settings</button> and choose <b>Export passwords</b>, then save the file.</li>
    <li><button class="btn small" data-file>Choose the exported file…</button></li></ol>
    <label class="row-flex small"><input type="checkbox" data-del checked> Securely delete the exported file afterwards (it contains your passwords in plain text)</label><div data-res class="small" style="margin-top:8px"></div></div>`);
  $('[data-chrome]', pw).onclick = () => call('passwords.openChromeExport').catch((e) => toast(e.message));
  $('[data-file]', pw).onclick = () => call('app.pickFile', { filter: 'CSV files|*.csv' }).then((p) => p && call('passwords.importCsv', { path: p, deleteFile: $('[data-del]', pw).checked }))
    .then((r) => { if (r) $('[data-res]', pw).textContent = `Added ${r.added}, updated ${r.updated}${r.skipped ? `, skipped ${r.skipped}` : ''}.${r.fileDeleted ? ' The export file was deleted.' : ''}`; }).catch((e) => toast(e.message));
  s.appendChild(card(pw));
}

async function system() {
  const s = section('system', 'System');
  const def = h('<button class="btn small">Make Solus your default browser</button>');
  def.onclick = () => call('app.makeDefault');
  s.appendChild(card(
    item('Default browser', 'Opens Windows Settings, where you can choose Solus for links and web files.', def),
    toggle('Keep Solus running in the notification area when its window is closed', 'Reminders and downloads keep working. Use "Exit Solus" from the menu or tray icon to quit completely.', 'close_to_tray', { local: true }),
    select('Free memory while hidden', 'Unload pages once the window has been in the notification area for:', 'tray_unload_after_min', [[2, '2 minutes'], [10, '10 minutes'], [30, '30 minutes'], [120, '2 hours']], { local: true, number: true }),
    toggle('Update Solus automatically', 'Updates are checked daily, verified with Solus\'s signing key, and installed when you restart.', 'update_check', { local: true }),
    toggle('Developer tools', 'Allow F12 / Inspect on web pages.', 'devtools'),
  ));
}

async function about() {
  const s = section('about', 'About Solus');
  const info = await call('info');
  const up = await call('update.status').catch(() => ({}));
  const btn = h('<button class="btn small">Check for updates</button>');
  btn.onclick = () => { btn.disabled = true; call('update.check').then((r) => toast(r.status === 'current' ? 'Solus is up to date.' : r.status === 'ready' ? `Version ${r.version} is ready — restart to install.` : r.error || 'Checking…')).catch((e) => toast(e.message)).finally(() => (btn.disabled = false)); };
  const folder = h('<button class="btn small">Open data folder</button>');
  folder.onclick = () => call('app.openPath', { path: info.dataDir });
  s.appendChild(card(
    item(`Solus ${esc(info.version)}`, `Engine: Chromium via Microsoft Edge WebView2 ${esc(info.engine)} — updated automatically by Windows.${up.ready ? `<br><b>Update ${esc(up.ready.version)} is ready.</b>` : ''}`, btn),
    item('Your data', esc(info.dataDir), folder),
    item('Open-source components', 'Chromium (WebView2), adblock-rust, EasyList/EasyPrivacy (CC BY-SA), uBlock filters (GPLv3), Fanboy lists, SQLite, Rust crates — see THIRD-PARTY-NOTICES.txt in the install folder.', null),
  ));
}

async function sitePage(origin) {
  const s = section('site', origin, 'Permissions and data for this site.');
  const perms = await call('permissions.forOrigin', { origin });
  const box = card(...perms.map((p) => {
    const sel = h(`<select class="field"><option value="default">Ask (default)</option><option value="allow"${p.state === 'allow' ? ' selected' : ''}>Allow</option><option value="block"${p.state === 'block' ? ' selected' : ''}>Block</option></select>`);
    sel.onchange = () => call('permissions.set', { origin, kind: p.kind, state: sel.value });
    return item(p.label, '', sel);
  }));
  const clr = h('<button class="btn small danger">Clear cookies and site data</button>');
  clr.onclick = () => call('sites.clearData', { origin }).then(() => toast('Cleared.')).catch((e) => toast(e.message));
  box.appendChild(item('Stored data', '', clr));
  s.appendChild(box);
}

// ---------------------------------------------------------------- page
const builders = { appearance, startup, search, tabs, privacy, passwords, ai, sync, performance, downloads, languages, shortcuts, profiles, import: importSec, system, about };

async function load(hash) {
  [S, L] = await Promise.all([call('getSettings'), call('app.local')]);
  const main = $('#main');
  main.innerHTML = '';
  restartNeeded = false;
  const site = decodeURIComponent((hash.match(/site=([^&]+)/) || [])[1] || '');
  if (site) { await sitePage(site); return; }
  for (const [id] of SECTIONS) await builders[id]();
  const target = (hash || '').replace('#', '');
  if (target) { const el = document.getElementById('sec-' + target); if (el) el.scrollIntoView(); }
}

function buildNav() {
  const nav = $('#nav');
  for (const [id, ic, label] of SECTIONS) {
    const a = h(`<a href="#${id}" data-id="${id}">${icon(ic, 17)}<span>${esc(label)}</span></a>`);
    a.onclick = (e) => { e.preventDefault(); const el = document.getElementById('sec-' + id); if (el) el.scrollIntoView({ behavior: 'smooth' }); history.replaceState(null, '', '#' + id); };
    nav.appendChild(a);
  }
  const obs = new IntersectionObserver((entries) => {
    for (const e of entries) if (e.isIntersecting) $$('.nav a').forEach((a) => a.classList.toggle('on', 'sec-' + a.dataset.id === e.target.id));
  }, { rootMargin: '-10% 0px -80% 0px' });
  new MutationObserver(() => $$('section.sec').forEach((s) => obs.observe(s))).observe($('#main'), { childList: true });
  $('#search').oninput = (e) => {
    const q = e.target.value.trim().toLowerCase();
    $$('.search-hit').forEach((x) => x.classList.remove('search-hit'));
    if (q.length < 2) return;
    const hit = $$('.item').find((x) => x.textContent.toLowerCase().includes(q));
    if (hit) { hit.classList.add('search-hit'); hit.scrollIntoView({ block: 'center' }); }
  };
}

(async () => {
  buildNav();
  // shortcut list comes from the browser UI's init data; the page asks the host directly
  try { window.__shortcuts = await call('shortcuts.list'); } catch { window.__shortcuts = []; }
  await load(location.hash);
  addEventListener('hashchange', () => { if (/site=/.test(location.hash)) load(location.hash); else { const el = document.getElementById('sec-' + location.hash.slice(1)); if (el) el.scrollIntoView(); } });
  solus.on('settings', (s) => { S = s; });
})();
