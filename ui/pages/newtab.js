// New tab page: search, shortcuts and customisable widgets. Layout lives in settings.newtab.
'use strict';

const DEFAULT = {
  sections: [{ id: 'clock', on: true }, { id: 'brand', on: false }, { id: 'search', on: true }, { id: 'shortcuts', on: true }, { id: 'widgets', on: true }],
  widgets: [{ id: 'w1', type: 'bookmarks' }, { id: 'w2', type: 'tasks' }, { id: 'w3', type: 'notes' }],
  shortcuts: { mostVisited: true, pinned: [], hidden: [], max: 8 },
  bg: { type: 'default', value: '' },
  align: 'center', name: '', h24: false,
};
const GRADIENTS = [
  'linear-gradient(135deg,#ffd8a8,#ff8787)', 'linear-gradient(135deg,#a5d8ff,#d0bfff)', 'linear-gradient(135deg,#b2f2bb,#66d9e8)',
  'linear-gradient(135deg,#1e3c72,#2a5298)', 'linear-gradient(135deg,#232526,#414345)', 'linear-gradient(135deg,#ff9a9e,#fecfef)',
  'linear-gradient(160deg,#0f2027,#203a43,#2c5364)',
];
const WIDGET_TYPES = { bookmarks: 'Bookmarks', tasks: 'Tasks', notes: 'Recent notes', history: 'Recently visited', weather: 'Weather', links: 'Links', note: 'Sticky note', rss: 'News feed' };
let cfg = structuredClone(DEFAULT);
let settings = {};

async function load() {
  settings = await call('getSettings');
  cfg = Object.assign(structuredClone(DEFAULT), settings.newtab && typeof settings.newtab === 'object' ? settings.newtab : {});
  cfg.shortcuts = Object.assign(structuredClone(DEFAULT.shortcuts), cfg.shortcuts || {});
  for (const s of DEFAULT.sections) if (!cfg.sections.find((x) => x.id === s.id)) cfg.sections.push({ ...s });
  render();
}
function save() { return call('setSetting', { key: 'newtab', value: cfg }); }

function applyBg() {
  const b = cfg.bg || {};
  document.body.classList.remove('img-bg');
  let v = '';
  if (b.type === 'gradient' || b.type === 'color') v = b.value;
  if (b.type === 'image' && /^https:\/\//.test(b.value)) { v = `center/cover no-repeat url("${b.value.replace(/"/g, '%22')}")`; document.body.classList.add('img-bg'); }
  document.body.style.background = v || '';
  document.body.classList.toggle('img-bg', b.type === 'image' || (b.type === 'gradient' && /1e3c72|232526|0f2027/.test(b.value)));
  document.body.classList.toggle('align-top', cfg.align === 'top');
}

function render() {
  applyBg();
  const wrap = $('#wrap');
  wrap.innerHTML = '';
  for (const s of cfg.sections) {
    if (!s.on) continue;
    const el = h(`<div class="section" data-sec="${s.id}"></div>`);
    wrap.appendChild(el);
    ({ clock, brand, search, shortcuts, widgets })[s.id]?.(el);
  }
}

// ---------------------------------------------------------------- sections
function clock(el) {
  const tick = () => {
    const d = new Date();
    const hr = d.getHours();
    const greet = hr < 5 ? 'Good night' : hr < 12 ? 'Good morning' : hr < 18 ? 'Good afternoon' : 'Good evening';
    el.innerHTML = `<div class="time">${d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: !cfg.h24 })}</div><div class="greet">${greet}${cfg.name ? ', ' + esc(cfg.name) : ''} · ${d.toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long' })}</div>`;
  };
  tick();
  clearInterval(window._clk); window._clk = setInterval(tick, 10000);
}
function brand(el) { el.innerHTML = `<div id="brand"><img src="/img/logo-256.png" alt="">Solus</div>`; }

function search(el) {
  const engine = (settings.engines || []).find((e) => e.id === settings.search_engine) || { name: 'Google' };
  el.innerHTML = `<div id="search"><div class="box">${icon('search', 20)}<input placeholder="Search ${esc(engine.name)} or type a URL" autofocus spellcheck="false" autocomplete="off"></div><div id="sugs" hidden></div></div>`;
  const box = $('#search', el), input = $('input', el), sugs = $('#sugs', el);
  let items = [], sel = -1, seq = 0, lastQ = '';
  const show = (list) => {
    items = list; sel = -1;
    sugs.hidden = !list.length; box.classList.toggle('open', !!list.length);
    sugs.innerHTML = list.map((it, i) => `<div class="sug" data-i="${i}">${icon(it.type === 'history' ? 'history' : it.type === 'bookmark' ? 'star' : it.type === 'url' ? 'globe' : 'search', 16)}<span>${esc(it.text || it.url)}</span>${it.type === 'history' || it.type === 'bookmark' ? `<span class="u">— ${esc(it.url)}</span>` : ''}</div>`).join('');
  };
  input.addEventListener('input', () => {
    const q = input.value.trim();
    if (!q) { show([]); return; }
    const s = ++seq; lastQ = q;
    call('suggest', { q }).then((r) => { if (s === seq) show(r.items || []); });
  });
  solus.on('suggestMore', (d) => { if (d.q === lastQ && input.value.trim() === lastQ) show(d.items); });
  const go = (it) => call('navigate', { input: it ? it.url : input.value.trim() });
  input.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown' && items.length) { sel = (sel + 1) % items.length; mark(); e.preventDefault(); }
    else if (e.key === 'ArrowUp' && items.length) { sel = (sel - 1 + items.length) % items.length; mark(); e.preventDefault(); }
    else if (e.key === 'Enter' && input.value.trim()) go(sel >= 0 ? items[sel] : null);
    else if (e.key === 'Escape') show([]);
  });
  const mark = () => { $$('.sug', sugs).forEach((x, i) => x.classList.toggle('sel', i === sel)); const it = items[sel]; if (it) input.value = it.type === 'search' || it.type === 'suggest' ? it.text : it.url; };
  sugs.addEventListener('mousedown', (e) => { const s = e.target.closest('.sug'); if (s) { e.preventDefault(); go(items[+s.dataset.i]); } });
  input.addEventListener('blur', () => setTimeout(() => show([]), 150));
  setTimeout(() => input.focus(), 50);
}

async function shortcuts(el) {
  const sc = cfg.shortcuts;
  let list = [...sc.pinned];
  if (sc.mostVisited) {
    try {
      const mv = await call('history.mostVisited', { limit: sc.max + sc.hidden.length + 4 });
      for (const m of mv) {
        if (list.length >= sc.max) break;
        if (sc.hidden.includes(m.url) || list.some((x) => sameSite(x.url, m.url))) continue;
        list.push({ title: m.title || host(m.url), url: m.url, auto: true });
      }
    } catch {}
  }
  list = list.slice(0, sc.max);
  const icons = await call('favicons.forUrls', { urls: list.map((x) => x.url) }).catch(() => ({}));
  el.innerHTML = `<div id="shortcuts">${list.map((x, i) => `<a class="sc" href="${esc(x.url)}" data-i="${i}" title="${esc(x.title)}\n${esc(x.url)}"><div class="ico">${icons[x.url] ? `<img src="${esc(icons[x.url])}" alt="">` : esc((x.title || host(x.url) || '?').slice(0, 1).toUpperCase())}</div><div class="name">${esc(x.title || host(x.url))}</div><button class="x" data-rm="${i}" title="${x.auto ? 'Don\'t show' : 'Remove'}">${icon('close', 13)}</button></a>`).join('')}
    ${list.length < sc.max ? `<a class="sc" href="#" data-add><div class="ico">${icon('plus', 22)}</div><div class="name">Add shortcut</div></a>` : ''}</div>`;
  el.onclick = (e) => {
    const rm = e.target.closest('[data-rm]');
    if (rm) {
      e.preventDefault();
      const x = list[+rm.dataset.rm];
      if (x.auto) sc.hidden.push(x.url); else sc.pinned = sc.pinned.filter((p) => p.url !== x.url);
      save().then(() => shortcuts(el));
      return;
    }
    if (e.target.closest('[data-add]')) { e.preventDefault(); addShortcut(el); }
  };
}
function host(u) { try { return new URL(u).hostname.replace(/^www\./, ''); } catch { return u; } }
function sameSite(a, b) { return host(a) === host(b); }
function addShortcut(el) {
  modal(`<h3>Add shortcut</h3><p class="muted small">Name</p><input class="field" data-n style="width:100%"><p class="muted small" style="margin-top:10px">URL</p><input class="field" data-u style="width:100%" placeholder="https://"><div class="actions"><button class="btn" data-x>Cancel</button><button class="btn primary" data-ok>Add</button></div>`, (m, close) => {
    $('[data-x]', m).onclick = () => close();
    $('[data-ok]', m).onclick = () => {
      let u = $('[data-u]', m).value.trim(); if (!u) return;
      if (!/^[a-z]+:\/\//i.test(u)) u = 'https://' + u;
      cfg.shortcuts.pinned.push({ title: $('[data-n]', m).value.trim() || host(u), url: u });
      save().then(() => { close(); shortcuts(el); });
    };
  });
}

// ---------------------------------------------------------------- widgets
function widgets(el) {
  el.innerHTML = `<div id="widgets"></div>`;
  const grid = $('#widgets', el);
  for (const w of cfg.widgets) {
    const card = h(`<div class="widget" data-w="${esc(w.id)}" draggable="true"><header>${icon(widgetIcon(w.type), 16)}<span class="t">${esc(w.title || WIDGET_TYPES[w.type] || 'Widget')}</span><button data-edit title="Widget settings">${icon('moreV', 15)}</button></header><div class="body"></div></div>`);
    grid.appendChild(card);
    const body = $('.body', card);
    (WIDGETS[w.type] || (() => (body.textContent = 'Unknown widget')))(body, w);
    $('[data-edit]', card).onclick = () => editWidget(w);
    card.addEventListener('dragstart', (e) => e.dataTransfer.setData('text/x-w', w.id));
    card.addEventListener('dragover', (e) => e.preventDefault());
    card.addEventListener('drop', (e) => {
      e.preventDefault();
      const from = cfg.widgets.findIndex((x) => x.id === e.dataTransfer.getData('text/x-w'));
      const to = cfg.widgets.findIndex((x) => x.id === w.id);
      if (from < 0 || from === to) return;
      cfg.widgets.splice(to, 0, cfg.widgets.splice(from, 1)[0]);
      save().then(render);
    });
  }
  const add = h(`<button class="widget" style="align-items:center;justify-content:center;color:var(--muted);border-style:dashed;cursor:default">${icon('plus', 22)}<span>Add widget</span></button>`);
  add.onclick = () => addWidget();
  grid.appendChild(add);
}
const widgetIcon = (t) => ({ bookmarks: 'star', tasks: 'tasks', notes: 'note', history: 'history', weather: 'sun', links: 'globe', note: 'edit', rss: 'layers' }[t] || 'grid');

const WIDGETS = {
  async bookmarks(body) {
    const kids = await call('bookmarks.children', { parent: 'bar' });
    const urls = kids.filter((k) => k.kind === 'url').slice(0, 8);
    body.innerHTML = urls.map((k) => `<a class="li" href="${esc(k.url)}">${k.icon ? `<img src="${esc(k.icon)}">` : icon('globe', 16)}<span>${esc(k.title || k.url)}</span></a>`).join('') || '<div class="muted small">Bookmarks on your bookmarks bar show here.</div>';
  },
  async history(body) {
    const rows = await call('history.query', { limit: 8 });
    body.innerHTML = rows.map((r) => `<a class="li" href="${esc(r.url)}">${r.icon ? `<img src="${esc(r.icon)}">` : icon('globe', 16)}<span>${esc(r.title || r.url)}</span></a>`).join('') || '<div class="muted small">No recent pages.</div>';
  },
  async tasks(body) {
    const draw = async () => {
      const list = (await call('tasks.list')).filter((t) => !t.done).slice(0, 7);
      body.innerHTML = `<input class="field" style="width:100%;height:30px;margin-bottom:6px" placeholder="Add a task and press Enter">` + (list.map((t) => `<label class="li"><input type="checkbox" data-id="${esc(t.id)}"><span>${esc(t.title)}</span>${t.due ? `<span class="muted small" style="margin-left:auto">${new Date(t.due).toLocaleDateString(undefined, { day: 'numeric', month: 'short' })}</span>` : ''}</label>`).join('') || '<div class="muted small">All done 🎉</div>');
      $('input.field', body).onkeydown = (e) => { if (e.key === 'Enter' && e.target.value.trim()) call('tasks.save', { title: e.target.value.trim() }).then(draw); };
      body.onchange = (e) => { const c = e.target.closest('[data-id]'); if (c) call('tasks.save', { id: c.dataset.id, done: true }).then(draw); };
    };
    draw();
  },
  async notes(body) {
    const list = (await call('notes.list', { q: '' })).slice(0, 5);
    body.innerHTML = list.map((n) => `<a class="li" href="${esc(n.scope === 'site' ? 'https://' + n.key : n.key)}" title="${esc(n.body)}">${icon('note', 15)}<span>${esc(n.body.split('\n')[0])}</span></a>`).join('') || '<div class="muted small">Notes you attach to web pages show here.</div>';
  },
  async weather(body, w) {
    if (!w.lat) { body.innerHTML = `<div class="muted small">Set a city in this widget's settings.</div>`; return; }
    body.innerHTML = '<span class="spinner"></span>';
    try {
      const r = await fetch(`https://api.open-meteo.com/v1/forecast?latitude=${w.lat}&longitude=${w.lon}&current=temperature_2m,weather_code,wind_speed_10m&daily=temperature_2m_max,temperature_2m_min,weather_code&timezone=auto&forecast_days=4`).then((x) => x.json());
      const c = r.current, d = r.daily;
      const desc = wxText(c.weather_code);
      body.innerHTML = `<div class="wx"><span>${icon(wxIcon(c.weather_code), 40)}</span><span class="deg">${Math.round(c.temperature_2m)}°</span><div><div>${esc(w.city)}</div><div class="muted small">${esc(desc)} · wind ${Math.round(c.wind_speed_10m)} km/h</div></div></div>
        <div style="display:flex;gap:14px;margin-top:10px" class="small">${d.time.slice(1).map((t, i) => `<div style="text-align:center"><div class="muted">${new Date(t).toLocaleDateString(undefined, { weekday: 'short' })}</div>${icon(wxIcon(d.weather_code[i + 1]), 18)}<div>${Math.round(d.temperature_2m_max[i + 1])}° / ${Math.round(d.temperature_2m_min[i + 1])}°</div></div>`).join('')}</div>`;
    } catch { body.innerHTML = '<div class="muted small">Weather is unavailable offline.</div>'; }
  },
  links(body, w) {
    body.innerHTML = (w.links || []).map((l) => `<a class="li" href="${esc(l.url)}">${icon('globe', 15)}<span>${esc(l.title || l.url)}</span></a>`).join('') || '<div class="muted small">Add links in this widget\'s settings.</div>';
  },
  note(body, w) {
    body.innerHTML = `<textarea placeholder="Type anything…">${esc(w.text || '')}</textarea>`;
    const ta = $('textarea', body);
    ta.oninput = () => { clearTimeout(ta._t); ta._t = setTimeout(() => { w.text = ta.value; save(); }, 600); };
  },
  async rss(body, w) {
    if (!w.url) { body.innerHTML = '<div class="muted small">Set a feed URL in this widget\'s settings.</div>'; return; }
    body.innerHTML = '<span class="spinner"></span>';
    try {
      const xml = new DOMParser().parseFromString(await call('fetchText', { url: w.url }), 'text/xml');
      const items = [...xml.querySelectorAll('item, entry')].slice(0, 6).map((it) => ({ title: it.querySelector('title')?.textContent || '', link: it.querySelector('link')?.getAttribute('href') || it.querySelector('link')?.textContent || '' }));
      body.innerHTML = items.filter((i) => /^https?:/.test(i.link)).map((i) => `<a class="li" href="${esc(i.link)}">${icon('chevRight', 14)}<span>${esc(i.title)}</span></a>`).join('') || '<div class="muted small">No items in this feed.</div>';
    } catch (e) { body.innerHTML = `<div class="muted small">Couldn't load the feed (${esc(e.message)}).</div>`; }
  },
};
const wxText = (c) => (c === 0 ? 'Clear' : c <= 3 ? 'Partly cloudy' : c <= 48 ? 'Fog' : c <= 57 ? 'Drizzle' : c <= 67 ? 'Rain' : c <= 77 ? 'Snow' : c <= 82 ? 'Showers' : c <= 86 ? 'Snow showers' : 'Thunderstorm');
const wxIcon = (c) => (c === 0 ? 'sun' : c <= 3 ? 'cloud' : c <= 48 ? 'cloud' : c <= 82 ? 'cloud' : 'bolt');

function addWidget() {
  modal(`<h3>Add widget</h3><div style="display:grid;grid-template-columns:1fr 1fr;gap:8px">${Object.entries(WIDGET_TYPES).map(([k, v]) => `<button class="btn" data-t="${k}" style="justify-content:flex-start">${icon(widgetIcon(k), 16)} ${esc(v)}</button>`).join('')}</div><div class="actions"><button class="btn" data-x>Close</button></div>`, (m, close) => {
    $('[data-x]', m).onclick = () => close();
    $$('[data-t]', m).forEach((b) => (b.onclick = () => {
      const w = { id: 'w' + Date.now().toString(36), type: b.dataset.t };
      cfg.widgets.push(w);
      save().then(() => { close(); render(); if (['weather', 'links', 'rss'].includes(w.type)) editWidget(w); });
    }));
  });
}

function editWidget(w) {
  const extra = w.type === 'weather' ? `<p class="muted small">City</p><input class="field" data-city style="width:100%" value="${esc(w.city || '')}" placeholder="e.g. Auckland">`
    : w.type === 'rss' ? `<p class="muted small">Feed URL (RSS or Atom)</p><input class="field" data-feed style="width:100%" value="${esc(w.url || '')}" placeholder="https://…/feed.xml">`
    : w.type === 'links' ? `<p class="muted small">Links — one per line as "Name | https://…"</p><textarea class="field" data-links rows="6" style="width:100%">${esc((w.links || []).map((l) => `${l.title} | ${l.url}`).join('\n'))}</textarea>` : '';
  modal(`<h3>${esc(WIDGET_TYPES[w.type])}</h3><p class="muted small">Title</p><input class="field" data-title style="width:100%" value="${esc(w.title || '')}" placeholder="${esc(WIDGET_TYPES[w.type])}">${extra}
    <div class="actions"><button class="btn danger" data-rm>Remove widget</button><span style="flex:1"></span><button class="btn" data-x>Cancel</button><button class="btn primary" data-ok>Save</button></div>`, (m, close) => {
    $('[data-x]', m).onclick = () => close();
    $('[data-rm]', m).onclick = () => { cfg.widgets = cfg.widgets.filter((x) => x.id !== w.id); save().then(() => { close(); render(); }); };
    $('[data-ok]', m).onclick = async () => {
      w.title = $('[data-title]', m).value.trim();
      if (w.type === 'weather') {
        const city = $('[data-city]', m).value.trim();
        if (city && city !== w.city) {
          try {
            const g = await fetch(`https://geocoding-api.open-meteo.com/v1/search?count=1&name=${encodeURIComponent(city)}`).then((r) => r.json());
            const r0 = g.results && g.results[0];
            if (!r0) { toast('City not found'); return; }
            Object.assign(w, { city: r0.name + (r0.country_code ? ', ' + r0.country_code : ''), lat: r0.latitude, lon: r0.longitude });
          } catch { toast('Could not look up the city (offline?)'); return; }
        }
      }
      if (w.type === 'rss') w.url = $('[data-feed]', m).value.trim();
      if (w.type === 'links') w.links = $('[data-links]', m).value.split('\n').map((l) => l.split('|').map((x) => x.trim())).filter((p) => p[1] || /^https?:/.test(p[0])).map((p) => ({ title: p[1] ? p[0] : '', url: p[1] || p[0] })).filter((l) => /^https?:\/\//.test(l.url));
      save().then(() => { close(); render(); });
    };
  });
}

// ---------------------------------------------------------------- customise panel
function customize() {
  if ($('.customize')) { $('.customize').remove(); return; }
  const p = h(`<div class="customize"><div style="display:flex;align-items:center"><h3 style="flex:1;margin:0">Customise new tab</h3><button class="icon-btn" data-x>${icon('close', 16)}</button></div>
    <h3>Sections</h3><div class="wlist" data-secs></div>
    <h3>Background</h3><div class="bgs" data-bgs></div>
    <div class="opt"><span>Solid colour</span><input type="color" data-color value="${cfg.bg.type === 'color' ? cfg.bg.value : '#f4f5f7'}"></div>
    <div class="opt" style="display:block"><div class="small muted">Image URL (https)</div><input class="field" data-img style="width:100%" value="${cfg.bg.type === 'image' ? esc(cfg.bg.value) : ''}" placeholder="https://images.example.com/photo.jpg"></div>
    <h3>Layout</h3>
    <div class="opt"><span>Position</span><div class="seg" data-align><button data-v="center">Centre</button><button data-v="top">Top</button></div></div>
    <div class="opt"><span>Your name (greeting)</span><input class="field" data-name style="width:150px" value="${esc(cfg.name || '')}"></div>
    <div class="opt"><span>24-hour clock</span>${sw(cfg.h24, 'data-h24')}</div>
    <h3>Shortcuts</h3>
    <div class="opt"><span>Include most visited sites</span>${sw(cfg.shortcuts.mostVisited, 'data-mv')}</div>
    <div class="opt"><span>Number of shortcuts</span><select class="field" data-max>${[4, 6, 8, 10, 12, 16].map((n) => `<option ${n === cfg.shortcuts.max ? 'selected' : ''}>${n}</option>`).join('')}</select></div>
    <div class="opt"><button class="btn small" data-unhide>Restore hidden shortcuts (${cfg.shortcuts.hidden.length})</button></div>
    <h3>Widgets</h3><div class="opt"><button class="btn small" data-addw>${icon('plus', 14)} Add widget</button><span class="muted small">Drag widgets to reorder</span></div>
    <h3>Reset</h3><button class="btn small danger" data-reset>Reset to default layout</button></div>`);
  document.body.appendChild(p);
  const names = { clock: 'Clock & greeting', brand: 'Solus logo', search: 'Search box', shortcuts: 'Shortcuts', widgets: 'Widgets' };
  const drawSecs = () => {
    $('[data-secs]', p).innerHTML = cfg.sections.map((s, i) => `<div class="opt" draggable="true" data-i="${i}">${icon('drag', 14)}<span style="flex:1">${names[s.id]}</span>${sw(s.on, `data-on="${i}"`)}</div>`).join('');
  };
  drawSecs();
  $('[data-secs]', p).onchange = (e) => { const i = e.target.dataset.on; if (i != null) { cfg.sections[+i].on = e.target.checked; save().then(render); } };
  $('[data-secs]', p).addEventListener('dragstart', (e) => e.dataTransfer.setData('text/x-s', e.target.closest('[data-i]').dataset.i));
  $('[data-secs]', p).addEventListener('dragover', (e) => e.preventDefault());
  $('[data-secs]', p).addEventListener('drop', (e) => {
    const to = e.target.closest('[data-i]'); if (!to) return;
    const from = +e.dataTransfer.getData('text/x-s');
    cfg.sections.splice(+to.dataset.i, 0, cfg.sections.splice(from, 1)[0]);
    save().then(() => { drawSecs(); render(); });
  });
  const bgs = $('[data-bgs]', p);
  bgs.innerHTML = `<button data-bg="" title="Default" style="background:var(--page);border-color:${cfg.bg.type === 'default' ? 'var(--accent)' : 'var(--border)'}"></button>` + GRADIENTS.map((g) => `<button data-bg="${g}" class="${cfg.bg.value === g ? 'on' : ''}" style="background:${g}"></button>`).join('');
  bgs.onclick = (e) => { const b = e.target.closest('[data-bg]'); if (!b) return; cfg.bg = b.dataset.bg ? { type: 'gradient', value: b.dataset.bg } : { type: 'default', value: '' }; save(); applyBg(); $$('button', bgs).forEach((x) => x.classList.toggle('on', x === b)); };
  $('[data-color]', p).oninput = (e) => { cfg.bg = { type: 'color', value: e.target.value }; applyBg(); clearTimeout(p._t); p._t = setTimeout(save, 400); };
  $('[data-img]', p).onchange = (e) => { const v = e.target.value.trim(); if (v && !/^https:\/\//.test(v)) { toast('Use an https:// image URL'); return; } cfg.bg = v ? { type: 'image', value: v } : { type: 'default', value: '' }; save(); applyBg(); };
  const al = $('[data-align]', p);
  $$('button', al).forEach((b) => { b.classList.toggle('on', b.dataset.v === (cfg.align || 'center')); b.onclick = () => { cfg.align = b.dataset.v; save(); applyBg(); $$('button', al).forEach((x) => x.classList.toggle('on', x === b)); }; });
  $('[data-name]', p).onchange = (e) => { cfg.name = e.target.value.trim().slice(0, 30); save().then(render); };
  $('[data-h24]', p).onchange = (e) => { cfg.h24 = e.target.checked; save().then(render); };
  $('[data-mv]', p).onchange = (e) => { cfg.shortcuts.mostVisited = e.target.checked; save().then(render); };
  $('[data-max]', p).onchange = (e) => { cfg.shortcuts.max = +e.target.value; save().then(render); };
  $('[data-unhide]', p).onclick = () => { cfg.shortcuts.hidden = []; save().then(() => { render(); p.remove(); customize(); }); };
  $('[data-addw]', p).onclick = () => addWidget();
  $('[data-reset]', p).onclick = () => confirmBox('Reset new tab page?', 'Your widgets, shortcuts and background will go back to the defaults.', 'Reset', true).then((ok) => { if (ok) { cfg = structuredClone(DEFAULT); save().then(() => { p.remove(); render(); }); } });
  $('[data-x]', p).onclick = () => p.remove();
}

$('#cust-btn').innerHTML = icon('edit', 16);
$('#cust-btn').onclick = customize;
solus.on('bookmarksChanged', () => render());
load().catch((e) => { $('#wrap').innerHTML = `<p class="muted">Could not load the new tab page: ${esc(e.message)}</p>`; });
