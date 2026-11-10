// Solus browser UI: tab strip, toolbar, omnibox, bookmarks bar, popups and layout reporting.
'use strict';

const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const h = (html) => { const t = document.createElement('template'); t.innerHTML = html.trim(); return t.content.firstElementChild; };
const call = (cmd, args) => solus.call(cmd, args);

const S = {
  wid: 0, incognito: false, settings: {}, theme: null, profile: {}, profiles: [], shortcuts: [],
  tabs: [], active: 0, groups: [], workspace: '', split: null, closedCount: 0,
  pageInfo: {}, workspaces: [], downloads: [], blocked: {}, findOpen: false, sidebar: null, aiOpen: false,
  windowActive: true,
};
window.S = S;

const GROUP_COLORS = ['blue', 'red', 'yellow', 'green', 'pink', 'purple', 'cyan', 'orange', 'grey'];
const gcolor = (c) => `var(--g-${GROUP_COLORS.includes(c) ? c : 'grey'})`;
const activeTab = () => S.tabs.find((t) => t.id === S.active);
const tabById = (id) => S.tabs.find((t) => t.id === id);
const kbd = (action) => (S.shortcuts.find((s) => s.action === action) || {}).keys || '';

// ------------------------------------------------------------------ static icons
function paintIcons() {
  $('#newtab-btn').innerHTML = icon('plus', 18);
  $('#tabsearch-btn').innerHTML = icon('chevDown', 16);
  $('#back-btn').innerHTML = icon('back', 18);
  $('#fwd-btn').innerHTML = icon('forward', 18);
  $('#reload-btn').innerHTML = icon('reload', 17);
  $('#home-btn').innerHTML = icon('home', 18);
  $('#notes-btn').innerHTML = icon('note', 16);
  $('#star-btn').innerHTML = icon('star', 17);
  $('#key-btn').innerHTML = icon('key', 16);
  $('#translate-btn').innerHTML = icon('translate', 16);
  $('#shield-btn').innerHTML = icon('shieldCheck', 18);
  $('#dl-btn').innerHTML = icon('download', 18);
  $('#ext-btn').innerHTML = icon('puzzle', 18);
  $('#ai-btn').innerHTML = icon('sparkle', 18);
  $('#sidebar-btn').innerHTML = icon('sidebar', 18);
  $('#menu-btn').innerHTML = icon('more', 18);
  $('#find-prev').innerHTML = icon('chevUp', 16);
  $('#find-next').innerHTML = icon('chevDown', 16);
  $('#find-close').innerHTML = icon('close', 16);
  $('#bm-other').innerHTML = icon('folder', 16) + '<span>Other bookmarks</span>';
}

// ------------------------------------------------------------------ host events
solus.on('init', (d) => {
  S.wid = d.wid; S.incognito = d.incognito; S.profile = d.profile; S.profiles = d.profiles;
  S.settings = d.settings; S.shortcuts = d.shortcuts; S.perf = d.perf;
  applyTheme(d.theme);
  applySettings();
  renderProfile();
  loadBookmarksBar();
  if (window.Panels) Panels.init();
  if (window.AI) AI.init();
  call('workspaces.list').then(setWorkspaces).catch(() => {});
  loadExtensions();
  call('downloads.list').then((l) => { S.downloads = l || []; renderDownloadsBtn(); }).catch(() => {});
});
solus.on('theme', applyTheme);
solus.on('settings', (s) => { S.settings = s; applySettings(); });
solus.on('tabs', (d) => {
  S.tabs = d.tabs; S.active = d.active; S.groups = d.groups || []; S.workspace = d.workspace || '';
  S.split = d.split; S.closedCount = d.closedCount;
  renderTabs(); renderToolbar(); renderPageHost(); renderWorkspaceBtn();
});
solus.on('tab', (t) => {
  const i = S.tabs.findIndex((x) => x.id === t.id);
  if (i < 0) return;
  S.tabs[i] = t;
  updateTabEl(t);
  if (t.id === S.active) { renderToolbar(); renderPageHost(); }
  else if (S.split && (S.split.a === t.id || S.split.b === t.id)) renderPageHost();
});
solus.on('active', (d) => { S.active = d.id; renderTabs(); renderToolbar(); renderPageHost(); if (S.findOpen) findRun(); });
solus.on('pageInfo', (d) => { S.pageInfo[d.tab] = d; if (d.tab === S.active) renderToolbar(); });
solus.on('focusOmnibox', () => focusOmnibox());
solus.on('contentFocus', () => closePopups());
solus.on('windowFocus', (d) => { S.windowActive = d.active; if (!d.active) closePopups(); document.body.classList.toggle('inactive', !d.active); });
solus.on('escape', () => { if (popups.length) closePopups(); else if (S.findOpen) closeFind(); });
solus.on('action', (d) => runUiAction(d.action, d.data || {}));
solus.on('findResult', (d) => { $('#find-count').textContent = d.count ? `${d.index + 1}/${d.count}` : ($('#find-input').value ? '0/0' : ''); });
solus.on('bookmarksChanged', () => { loadBookmarksBar(); if (window.Panels) Panels.refresh('bookmarks'); });
solus.on('duplicate', (d) => {
  const other = tabById(d.other);
  if (!other || d.tab !== S.active) return;
  infobar('dup-' + d.tab, { icon: 'copy', html: `This page is already open in another tab: <b>${esc(other.title)}</b>`,
    buttons: [{ label: 'Switch to it', primary: true, run: () => { call('closeTab', { id: d.tab }); call('activate', { id: d.other }); } },
              { label: 'Keep both', run: () => {} }] });
});
solus.on('workspaces', setWorkspaces);
solus.on('suggestMore', (d) => Omni.more(d));
solus.on('infobar', (d) => infobar(d.id, { icon: d.icon || 'info', html: esc(d.text), buttons: (d.buttons || []).map((b) => ({ label: b.label, primary: b.primary, run: () => call(b.cmd, b.args || {}) })) }));
solus.on('toast', (d) => toast(d.text));
solus.on('panel', (d) => window.Panels && Panels.open(d.panel));
solus.on('downloads', (l) => { S.downloads = l; renderDownloadsBtn(); if (window.Panels) Panels.refresh('downloads'); });
solus.on('blocked', (d) => { S.blocked[d.tab] = d.count; if (d.tab === S.active) renderShield(); });
solus.on('permissionPrompt', (d) => {
  const el = infobar('perm-' + d.id, { icon: d.kind === 'camera' ? 'eye' : d.kind === 'notifications' ? 'bell' : 'shield', html: '<b>' + esc(d.host) + '</b> wants to ' + esc(d.label.charAt(0).toLowerCase() + d.label.slice(1)),
    buttons: [{ label: 'Block', run: () => call('permissions.respond', { id: d.id, allow: false }) }, { label: 'Allow', primary: true, run: () => call('permissions.respond', { id: d.id, allow: true }) }],
    onClose: () => call('permissions.respond', { id: d.id, allow: false, remember: false }) });
  el.dataset.tab = String(d.tab); syncInfobars();
});
solus.on('passwordPrompt', (d) => {
  const el = infobar('pw', { icon: 'key', html: (d.update ? 'Update' : 'Save') + ' password for <b>' + esc(d.user || '(no username)') + '</b> on ' + esc(d.host) + '?',
    buttons: [{ label: 'Never for this site', run: () => call('passwords.respond', { choice: 'never' }) }, { label: d.update ? 'Update' : 'Save', primary: true, run: () => call('passwords.respond', { choice: 'save' }) }],
    onClose: () => call('passwords.respond', { choice: 'dismiss' }) });
  el.dataset.tab = String(d.tab); syncInfobars();
});
solus.on('extensionsChanged', () => loadExtensions());
solus.on('perf', (d) => { S.perf = d.profile; });
function loadExtensions() { call('ext.list').then((l) => { $('#ext-btn').hidden = S.incognito || !l.some((x) => x.enabled); requestAnimationFrame(reportLayout); }).catch(() => {}); }

function applyTheme(t) {
  if (!t) return;
  S.theme = t;
  applySolusTheme(t);
  const cs = getComputedStyle(document.documentElement);
  const v = (k) => cs.getPropertyValue(k).trim();
  call('colors', { bg: v('--frame'), fg: v('--text'), hover: v('--hover-strong'), inactive: v('--muted') }).catch(() => {});
  requestAnimationFrame(reportLayout);
}

function applySettings() {
  const s = S.settings;
  $('#home-btn').hidden = !s.show_home_button;
  const engine = (s.engines || []).find((e) => e.id === s.search_engine);
  $('#omni-input').placeholder = `Search ${engine ? engine.name : 'Google'} or type a URL`;
  if (window.Panels) Panels.applySettings();
  requestAnimationFrame(reportLayout);
}

// ------------------------------------------------------------------ layout reporting
let lastLayout = '';
function reportLayout() {
  const host = $('#page-host').getBoundingClientRect();
  const insets = { top: host.top, left: host.left, right: innerWidth - host.right, bottom: innerHeight - host.bottom };
  const overlays = popups.filter((p) => p.el.isConnected && !p.passive).map((p) => {
    const r = p.el.getBoundingClientRect();
    return [r.left, r.top, r.width, r.height];
  });
  // hover cards and toasts are visual only but must still show over the page
  $$('.hovercard, .toast, .ai-float').forEach((el) => { const r = el.getBoundingClientRect(); overlays.push([r.left, r.top, r.width, r.height]); });
  const captionH = $('#tabstrip').getBoundingClientRect().height;
  const payload = { insets, overlays, captionH };
  if (S.splitDrag) payload.splitRatio = S.splitDrag;
  const key = JSON.stringify(payload);
  if (key === lastLayout) return;
  lastLayout = key;
  call('layout', payload).catch(() => {});
}
new ResizeObserver(() => { reportLayout(); renderPageHost(); fitTabs(); }).observe($('#page-host'));
addEventListener('resize', () => { reportLayout(); closePopups(); fitTabs(); });

// ------------------------------------------------------------------ popups
const layer = $('#layer');
let popups = [];

function openPopup(el, o = {}) {
  if (!o.keep) closePopups(o.keepPassive ? (p) => p.passive : null);
  layer.appendChild(el);
  const entry = { el, onClose: o.onClose, passive: !!o.passive, owner: o.owner, level: o.level || 0 };
  popups.push(entry);
  place(el, o);
  if (o.focus !== false && !o.passive) setTimeout(() => (el.querySelector('input,textarea,[autofocus]') || el).focus?.(), 0);
  reportLayout();
  return entry;
}

function place(el, o) {
  el.style.left = '0px'; el.style.top = '0px';
  const r = el.getBoundingClientRect();
  let x = o.x ?? 0, y = o.y ?? 0;
  if (o.anchor) {
    const a = o.anchor.getBoundingClientRect ? o.anchor.getBoundingClientRect() : o.anchor;
    if (o.side === 'right') { x = a.right - 2; y = a.top - 5; }
    else { x = o.align === 'right' ? a.right - r.width : a.left; y = a.bottom + (o.gap ?? 4); }
  }
  if (x + r.width > innerWidth - 6) x = o.side === 'right' && o.anchor ? (o.anchor.getBoundingClientRect().left - r.width + 2) : innerWidth - r.width - 6;
  if (y + r.height > innerHeight - 6) y = Math.max(6, innerHeight - r.height - 6);
  el.style.left = Math.max(4, x) + 'px';
  el.style.top = Math.max(4, y) + 'px';
}

function closePopups(keep) {
  const remaining = [];
  for (const p of popups) {
    if (keep && keep(p)) { remaining.push(p); continue; }
    p.el.remove();
    try { p.onClose && p.onClose(); } catch (e) { console.error(e); }
  }
  popups = remaining;
  reportLayout();
}
function closePopup(el) {
  const i = popups.findIndex((p) => p.el === el);
  if (i < 0) return;
  // close it and any popups opened after it (submenus)
  const gone = popups.splice(i);
  gone.forEach((p) => { p.el.remove(); try { p.onClose && p.onClose(); } catch (e) {} });
  reportLayout();
}

document.addEventListener('mousedown', (e) => {
  if (!popups.length) return;
  if (popups.some((p) => p.el.contains(e.target))) return;
  if (e.target.closest('[data-popup-owner]') && popups.some((p) => p.owner === e.target.closest('[data-popup-owner]'))) return;
  closePopups();
}, true);

// Menu: items = [{label, icon, img, kbd, run, disabled, checked, sub}, '-', {head}, {zoom: true}]
function menu(items, o = {}) {
  const el = h('<div class="popup menu" tabindex="-1" role="menu"></div>');
  let sel = -1;
  const rows = [];
  for (const it of items) {
    if (!it) continue;
    if (it === '-') { el.appendChild(h('<div class="sep"></div>')); continue; }
    if (it.head) { el.appendChild(h(`<div class="mhead">${esc(it.head)}</div>`)); continue; }
    if (it.el) { el.appendChild(it.el); continue; }
    const slot = it.img ? `<img src="${esc(it.img)}" alt="">` : it.checked ? icon('check', 15) : it.icon ? icon(it.icon, 16) : '';
    const row = h(`<div class="mi${it.disabled ? ' disabled' : ''}" role="menuitem"><span class="ic-slot">${slot}</span><span class="lbl">${esc(it.label)}</span>${it.kbd ? `<span class="kbd">${esc(it.kbd)}</span>` : ''}${it.sub ? icon('chevRight', 14) : ''}</div>`);
    if (it.title) row.title = it.title;
    rows.push({ row, it });
    el.appendChild(row);
    const activate = () => {
      if (it.disabled) return;
      if (it.sub) { openSub(row, it); return; }
      closePopups();
      it.run && it.run();
    };
    row.addEventListener('click', activate);
    row.addEventListener('auxclick', (e) => { if (e.button === 1 && it.middle) { closePopups(); it.middle(); } });
    row.addEventListener('contextmenu', (e) => { e.preventDefault(); if (it.context) it.context(e); });
    row.addEventListener('mouseenter', () => {
      setSel(rows.findIndex((r) => r.row === row));
      clearTimeout(el._subT);
      if (it.sub) el._subT = setTimeout(() => openSub(row, it), 180);
      else closeSubsOf(el);
    });
  }
  function setSel(i) { sel = i; rows.forEach((r, j) => r.row.classList.toggle('sel', j === i)); }
  function openSub(row, it) {
    closeSubsOf(el);
    const kids = typeof it.sub === 'function' ? it.sub() : it.sub;
    Promise.resolve(kids).then((k) => {
      const sub = menu(k, { level: (o.level || 0) + 1 });
      el._child = sub;
      openPopup(sub, { anchor: row, side: 'right', keep: true, level: (o.level || 0) + 1 });
    });
  }
  function closeSubsOf(m) { if (m._child) { closePopup(m._child); m._child = null; } }
  el.addEventListener('keydown', (e) => {
    const enabled = rows.map((r, i) => (r.it.disabled ? -1 : i)).filter((i) => i >= 0);
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const cur = enabled.indexOf(sel);
      const next = e.key === 'ArrowDown' ? enabled[(cur + 1) % enabled.length] : enabled[(cur - 1 + enabled.length) % enabled.length];
      setSel(next); rows[next]?.row.scrollIntoView({ block: 'nearest' });
    } else if (e.key === 'Enter' && sel >= 0) { e.preventDefault(); rows[sel].row.click(); }
    else if (e.key === 'ArrowRight' && sel >= 0 && rows[sel].it.sub) { openSub(rows[sel].row, rows[sel].it); }
    else if (e.key === 'ArrowLeft' && (o.level || 0) > 0) { closePopup(el); }
    else if (e.key === 'Escape') { closePopups(); }
  });
  return el;
}
function showMenu(items, o) { const el = menu(items, o); openPopup(el, o); return el; }

function toast(text, action) {
  $$('.toast').forEach((t) => t.remove());
  const t = h(`<div class="toast">${esc(text)}</div>`);
  if (action) { const b = h(`<button>${esc(action.label)}</button>`); b.onclick = () => { t.remove(); reportLayout(); action.run(); }; t.appendChild(b); }
  layer.appendChild(t);
  reportLayout();
  setTimeout(() => { t.remove(); reportLayout(); }, action ? 7000 : 3500);
}

// ------------------------------------------------------------------ tab strip
const tabsEl = $('#tabs');
let drag = null;

function visibleTabs() { return S.tabs.filter((t) => (t.workspace || '') === (S.workspace || '')); }

function favHtml(t) {
  if (t.loading && !t.discarded) return '<div class="spinner"></div>';
  if (t.crashed) return icon('crash', 16);
  if (t.url.startsWith('solus://')) return `<img src="/img/logo-256.png" alt="">`;
  if (t.favicon) return `<img src="${esc(t.favicon)}" alt="">`;
  return icon('globe', 16);
}

function tabHtml(t) {
  const g = t.group && S.groups.find((x) => x.id === t.group);
  const cls = ['tab', t.id === S.active ? 'active' : '', t.pinned ? 'pinned' : '', t.discarded ? 'unloaded' : '', t.crashed ? 'crashed' : ''].join(' ');
  const audio = t.audible || t.muted ? `<button class="audio" data-act="mute" title="${t.muted ? 'Unmute' : 'Mute'} tab">${icon(t.muted ? 'mute' : 'volume', 14)}</button>` : '';
  return `<div class="${cls}" data-id="${t.id}" role="tab" aria-selected="${t.id === S.active}">
    <span class="fav">${favHtml(t)}</span><span class="title">${esc(t.title)}</span>${audio}
    <button class="close" data-act="close" title="Close tab (${esc(kbd('closeTab'))})">${icon('close', 14)}</button>
    <span class="curve-r"></span>${g ? `<span class="gline" style="background:${gcolor(g.color)}"></span>` : ''}</div>`;
}

function renderTabs() {
  if (drag) { drag.pendingRender = true; return; }
  const list = visibleTabs();
  let html = '';
  let lastGroup = '';
  for (const t of list) {
    if (t.group && t.group !== lastGroup) {
      const g = S.groups.find((x) => x.id === t.group);
      if (g) {
        const n = list.filter((x) => x.group === g.id).length;
        html += `<div class="group-chip${g.collapsed ? ' collapsed' : ''}" data-group="${esc(g.id)}" style="background:${gcolor(g.color)}" title="${esc(g.name)} — click to ${g.collapsed ? 'expand' : 'collapse'}">${esc(g.name || ' ')}${g.collapsed ? `<span class="count">${n}</span>` : ''}</div>`;
      }
    }
    lastGroup = t.group || '';
    const g = t.group && S.groups.find((x) => x.id === t.group);
    if (g && g.collapsed && t.id !== S.active) continue;
    html += tabHtml(t);
  }
  tabsEl.innerHTML = html;
  const act = tabsEl.querySelector('.tab.active');
  if (act) {
    const prev = act.previousElementSibling;
    if (prev && prev.classList.contains('tab')) prev.classList.add('before-active');
    act.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }
  fitTabs();
  renderWorkspaceBtn();
}

function updateTabEl(t) {
  const el = tabsEl.querySelector(`.tab[data-id="${t.id}"]`);
  if (!el) { renderTabs(); return; }
  const tmp = h(tabHtml(t));
  if (el.classList.contains('before-active')) tmp.classList.add('before-active');
  if (el.classList.contains('narrow')) tmp.classList.add('narrow');
  el.replaceWith(tmp);
}

function fitTabs() {
  const tabs = $$('.tab:not(.pinned)', tabsEl);
  if (!tabs.length) return;
  const strip = $('#tabstrip').getBoundingClientRect().width;
  const fixed = $('#ws-btn').offsetWidth + $('#newtab-btn').offsetWidth + $('#tabsearch-btn').offsetWidth + 138 + 40;
  const pinnedW = $$('.tab.pinned', tabsEl).length * 44 + $$('.group-chip', tabsEl).reduce((a, c) => a + c.offsetWidth + 8, 0);
  const each = (strip - fixed - pinnedW) / tabs.length;
  tabs.forEach((el) => el.classList.toggle('narrow', each < 76));
}

tabsEl.addEventListener('mousedown', (e) => {
  const tabEl = e.target.closest('.tab');
  if (!tabEl) return;
  const id = +tabEl.dataset.id;
  if (e.button === 1) { e.preventDefault(); return; }
  if (e.button !== 0 || e.target.closest('button')) return;
  if (id !== S.active) { S.active = id; renderTabs(); call('activate', { id }); }
  drag = { id, x0: e.clientX, started: false, el: null };
});
tabsEl.addEventListener('auxclick', (e) => {
  const tabEl = e.target.closest('.tab');
  if (tabEl && e.button === 1) call('closeTab', { id: +tabEl.dataset.id });
});
tabsEl.addEventListener('click', (e) => {
  const chip = e.target.closest('.group-chip');
  if (chip) { call('groups.update', { id: chip.dataset.group, collapsed: !S.groups.find((g) => g.id === chip.dataset.group)?.collapsed }); return; }
  const tabEl = e.target.closest('.tab');
  if (!tabEl) return;
  const id = +tabEl.dataset.id;
  const act = e.target.closest('[data-act]')?.dataset.act;
  if (act === 'close') call('closeTab', { id });
  else if (act === 'mute') { const t = tabById(id); call('tabAction', { id, action: t.muted ? 'unmute' : 'mute' }); }
});
tabsEl.addEventListener('dblclick', (e) => { if (!e.target.closest('.tab,.group-chip')) call('window', { action: 'maximize' }); });
tabsEl.addEventListener('contextmenu', (e) => {
  e.preventDefault();
  const chip = e.target.closest('.group-chip');
  if (chip) return groupMenu(chip.dataset.group, { x: e.clientX, y: e.clientY });
  const tabEl = e.target.closest('.tab');
  if (tabEl) tabMenu(+tabEl.dataset.id, { x: e.clientX, y: e.clientY });
});
tabsEl.addEventListener('wheel', (e) => { tabsEl.scrollLeft += e.deltaY; }, { passive: true });

document.addEventListener('mousemove', (e) => {
  if (!drag) return;
  if (!drag.started) {
    if (Math.abs(e.clientX - drag.x0) < 6) return;
    drag.started = true;
    drag.el = tabsEl.querySelector(`.tab[data-id="${drag.id}"]`);
    if (!drag.el) { drag = null; return; }
    drag.el.classList.add('dragging');
    hideHoverCard();
  }
  drag.el.style.transform = `translateX(${e.clientX - drag.x0}px)`;
  const others = $$('.tab', tabsEl).filter((x) => x !== drag.el);
  let target = null;
  for (const o of others) { const r = o.getBoundingClientRect(); if (e.clientX < r.left + r.width / 2) { target = o; break; } }
  drag.target = target ? +target.dataset.id : null;
  let marker = $('.drop-marker', tabsEl.parentElement);
  if (!marker) { marker = h('<div class="drop-marker"></div>'); tabsEl.parentElement.appendChild(marker); }
  const ref = target ? target.getBoundingClientRect().left : (others.length ? others[others.length - 1].getBoundingClientRect().right : 0);
  marker.style.left = (ref - $('#tabstrip').getBoundingClientRect().left - 1) + 'px';
});
document.addEventListener('mouseup', () => {
  if (!drag) return;
  const d = drag; drag = null;
  $('.drop-marker')?.remove();
  if (d.started) {
    const from = S.tabs.findIndex((t) => t.id === d.id);
    let to;
    if (d.target != null) { to = S.tabs.findIndex((t) => t.id === d.target); if (from < to) to -= 1; }
    else { const vis = visibleTabs(); const last = vis[vis.length - 1]; to = S.tabs.findIndex((t) => t.id === last.id); if (from > to) to += 1; }
    const moved = S.tabs.splice(from, 1)[0];
    S.tabs.splice(to, 0, moved);
    // dropping next to a grouped tab joins that group; between groups leaves it
    const nb = S.tabs[to + 1] || S.tabs[to - 1];
    const prevT = S.tabs[to - 1], nextT = S.tabs[to + 1];
    const grp = prevT && nextT && prevT.group && prevT.group === nextT.group ? prevT.group : (moved.group && ((prevT && prevT.group === moved.group) || (nextT && nextT.group === moved.group)) ? moved.group : '');
    if (grp !== (moved.group || '')) call('groups.assign', { ids: [moved.id], group: grp });
    void nb;
    call('moveTab', { id: d.id, to });
  }
  renderTabs();
});

// hover cards
let hoverT = null;
tabsEl.addEventListener('mouseover', (e) => {
  const tabEl = e.target.closest('.tab');
  if (!tabEl || drag || S.settings.tab_hover_cards === false) return;
  clearTimeout(hoverT);
  hoverT = setTimeout(() => showHoverCard(+tabEl.dataset.id, tabEl), $('.hovercard') ? 60 : 650);
});
tabsEl.addEventListener('mouseleave', hideHoverCard);
tabsEl.addEventListener('mousedown', hideHoverCard);
function showHoverCard(id, el) {
  const t = tabById(id);
  if (!t || !el.isConnected) return;
  hideHoverCard(true);
  const state = t.crashed ? 'Crashed — click to reload' : t.discarded ? 'Unloaded to save memory' : t.frozen ? 'Paused to save memory' : t.audible ? 'Playing audio' : '';
  const card = h(`<div class="popup hovercard"><div class="t">${esc(t.title)}</div><div class="u">${esc(displayUrl(t.url))}</div>${state ? `<div class="m">${icon(t.discarded || t.frozen ? 'leaf' : t.audible ? 'volume' : 'info', 13)}${esc(state)}</div>` : ''}</div>`);
  layer.appendChild(card);
  const r = el.getBoundingClientRect();
  card.style.left = Math.min(r.left, innerWidth - 270) + 'px';
  card.style.top = (r.bottom + 4) + 'px';
  reportLayout();
}
function hideHoverCard(keepTimer) {
  if (keepTimer !== true) clearTimeout(hoverT);
  const had = $$('.hovercard');
  had.forEach((c) => c.remove());
  if (had.length) reportLayout();
}

function tabMenu(id, pos) {
  const t = tabById(id);
  if (!t) return;
  const idx = S.tabs.indexOf(t);
  const groups = S.groups.filter((g) => S.tabs.some((x) => x.group === g.id && (x.workspace || '') === (S.workspace || '')));
  const others = visibleTabs().filter((x) => x.id !== id && !x.pinned);
  showMenu([
    { label: 'New tab to the right', icon: 'plus', run: () => call('newTab', { index: idx + 1, group: t.group }) },
    '-',
    { label: 'Reload', icon: 'reload', run: () => call('tabAction', { id, action: 'reload' }) },
    { label: 'Duplicate', icon: 'copy', run: () => call('duplicateTab', { id }) },
    { label: t.pinned ? 'Unpin' : 'Pin', icon: 'pin', run: () => call('pinTab', { id, pinned: !t.pinned }) },
    { label: t.muted ? 'Unmute site' : 'Mute tab', icon: t.muted ? 'volume' : 'mute', run: () => call('tabAction', { id, action: t.muted ? 'unmute' : 'mute' }) },
    '-',
    { label: 'Add tab to group', icon: 'layers', sub: () => [
      { label: 'New group', icon: 'plus', run: () => newGroupFor([id]) },
      ...(groups.length ? ['-'] : []),
      ...groups.map((g) => ({ label: g.name || 'Unnamed group', run: () => call('groups.assign', { ids: [id], group: g.id }) })),
    ] },
    t.group ? { label: 'Remove from group', run: () => call('groups.assign', { ids: [id], group: '' }) } : null,
    { label: 'Move to workspace', icon: 'grid', sub: () => S.workspaces.filter((w) => w.id !== (t.workspace || '')).map((w) => ({ label: w.name, run: () => call('workspaces.moveTabs', { ids: [id], workspace: w.id }) })).concat([{ label: 'New workspace…', icon: 'plus', run: () => newWorkspace([id]) }]) },
    id !== S.active ? { label: 'Open in split view with current tab', icon: 'split', run: () => call('split', { with: id }) } : { label: S.split ? 'Exit split view' : 'Split view', icon: 'split', run: () => call(S.split ? 'unsplit' : 'split', {}) },
    !t.discarded && id !== S.active ? { label: 'Unload tab to free memory', icon: 'leaf', run: () => call('perf.discard', { id }) } : null,
    '-',
    { label: 'Close', icon: 'close', kbd: kbd('closeTab'), run: () => call('closeTab', { id }) },
    { label: 'Close other tabs', disabled: !others.length, run: () => call('closeTabs', { ids: others.map((x) => x.id) }) },
    { label: 'Close tabs to the right', run: () => { const vis = visibleTabs(); const i = vis.findIndex((x) => x.id === id); call('closeTabs', { ids: vis.slice(i + 1).filter((x) => !x.pinned).map((x) => x.id) }); } },
  ], pos);
}

function groupMenu(gid, pos) {
  const g = S.groups.find((x) => x.id === gid);
  if (!g) return;
  const editor = h(`<div style="padding:6px 8px 8px"><input class="field" style="width:100%" value="${esc(g.name)}" placeholder="Name this group"><div class="swatches">${GROUP_COLORS.map((c) => `<button class="swatch${c === g.color ? ' on' : ''}" data-c="${c}" style="background:${gcolor(c)}" title="${c}"></button>`).join('')}</div></div>`);
  const inp = $('input', editor);
  inp.addEventListener('keydown', (e) => { if (e.key === 'Enter') { call('groups.update', { id: gid, name: inp.value }); closePopups(); } e.stopPropagation(); });
  inp.addEventListener('change', () => call('groups.update', { id: gid, name: inp.value }));
  $$('.swatch', editor).forEach((b) => b.addEventListener('click', () => { call('groups.update', { id: gid, color: b.dataset.c }); $$('.swatch', editor).forEach((x) => x.classList.toggle('on', x === b)); }));
  const ids = S.tabs.filter((t) => t.group === gid).map((t) => t.id);
  showMenu([
    { el: editor },
    '-',
    { label: 'New tab in group', icon: 'plus', run: () => { const last = S.tabs.map((t) => t.group).lastIndexOf(gid); call('newTab', { index: last + 1, group: gid }); } },
    { label: g.collapsed ? 'Expand group' : 'Collapse group', icon: 'chevDown', run: () => call('groups.update', { id: gid, collapsed: !g.collapsed }) },
    { label: 'Ungroup', icon: 'layers', run: () => call('groups.remove', { id: gid }) },
    { label: 'Move group to workspace', icon: 'grid', sub: () => S.workspaces.filter((w) => w.id !== (S.workspace || '')).map((w) => ({ label: w.name, run: () => call('workspaces.moveTabs', { ids, workspace: w.id }) })) },
    '-',
    { label: 'Close group', icon: 'close', run: () => call('closeTabs', { ids }) },
  ], pos);
}

function newGroupFor(ids) {
  call('groups.create', { ids, name: '', color: GROUP_COLORS[S.groups.length % 8] }).then((gid) => {
    setTimeout(() => { const chip = $(`.group-chip[data-group="${CSS.escape(gid)}"]`); if (chip) { const r = chip.getBoundingClientRect(); groupMenu(gid, { x: r.left, y: r.bottom + 4 }); } }, 80);
  });
}

$('#newtab-btn').addEventListener('click', () => call('newTab', {}));
$('#newtab-btn').addEventListener('contextmenu', (e) => { e.preventDefault(); closedMenu({ x: e.clientX, y: e.clientY }); });
$('#tabsearch-btn').addEventListener('click', () => tabSearch());

function closedMenu(pos) {
  call('closedTabs').then((list) => {
    showMenu(list.length ? [{ head: 'Recently closed' }, ...list.slice(0, 15).map((c, i) => ({ label: c.title || c.url, img: c.favicon || null, icon: c.favicon ? null : 'globe', run: () => call('reopenClosedAt', { index: i }) }))] : [{ label: 'No recently closed tabs', disabled: true }], pos);
  });
}

function tabSearch() {
  const el = h(`<div class="popup tabsearch"><input placeholder="Search open and recently closed tabs" spellcheck="false"><div class="list"></div></div>`);
  const input = $('input', el), list = $('.list', el);
  let items = [], sel = 0, closed = [];
  call('closedTabs').then((c) => { closed = c; render(); });
  function render() {
    const q = input.value.toLowerCase();
    const wsName = (w) => (S.workspaces.find((x) => x.id === (w || '')) || {}).name || '';
    const open = S.tabs.filter((t) => !q || t.title.toLowerCase().includes(q) || t.url.toLowerCase().includes(q)).sort((a, b) => b.lastActive - a.lastActive);
    const cl = closed.filter((t) => q && ((t.title || '').toLowerCase().includes(q) || t.url.toLowerCase().includes(q))).slice(0, 5);
    items = [...open.map((t) => ({ kind: 'open', t })), ...cl.map((t, i) => ({ kind: 'closed', t, i: closed.indexOf(t) }))];
    sel = Math.min(sel, Math.max(0, items.length - 1));
    list.innerHTML = items.length ? items.map((it, i) => `<div class="row${i === sel ? ' sel' : ''}" data-i="${i}">${it.t.favicon ? `<img src="${esc(it.t.favicon)}">` : icon('globe', 16)}<div class="stack"><div class="t">${esc(it.t.title || it.t.url)}</div><div class="sub">${it.kind === 'closed' ? 'Recently closed · ' : (it.t.workspace || '') !== (S.workspace || '') ? esc(wsName(it.t.workspace)) + ' · ' : ''}${esc(displayUrl(it.t.url))}</div></div>${it.kind === 'open' ? `<span class="acts"><button data-close="${it.t.id}" title="Close">${icon('close', 14)}</button></span>` : ''}</div>`).join('') : '<div class="empty">No matching tabs</div>';
  }
  function choose(i) {
    const it = items[i]; if (!it) return;
    closePopups();
    if (it.kind === 'open') call('activate', { id: it.t.id }); else call('reopenClosedAt', { index: it.i });
  }
  input.addEventListener('input', () => { sel = 0; render(); });
  input.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown') { sel = Math.min(items.length - 1, sel + 1); render(); e.preventDefault(); }
    else if (e.key === 'ArrowUp') { sel = Math.max(0, sel - 1); render(); e.preventDefault(); }
    else if (e.key === 'Enter') choose(sel);
    else if (e.key === 'Escape') closePopups();
  });
  list.addEventListener('click', (e) => {
    const c = e.target.closest('[data-close]');
    if (c) { call('closeTab', { id: +c.dataset.close }); setTimeout(render, 120); return; }
    const r = e.target.closest('.row'); if (r) choose(+r.dataset.i);
  });
  render();
  openPopup(el, { anchor: $('#tabsearch-btn'), align: 'right' });
}

// ------------------------------------------------------------------ workspaces
function setWorkspaces(list) {
  S.workspaces = list || [];
  renderWorkspaceBtn();
}
function renderWorkspaceBtn() {
  const w = S.workspaces.find((x) => x.id === (S.workspace || '')) || { name: 'Personal', color: '' };
  const btn = $('#ws-btn');
  btn.hidden = S.incognito;
  $('.ws-name', btn).textContent = w.name;
  $('.ws-dot', btn).style.background = w.color || 'var(--accent)';
  btn.title = `Workspace: ${w.name}`;
}
$('#ws-btn').addEventListener('click', () => workspaceMenu());
function workspaceMenu() {
  const counts = {};
  S.tabs.forEach((t) => { counts[t.workspace || ''] = (counts[t.workspace || ''] || 0) + 1; });
  showMenu([
    { head: 'Workspaces' },
    ...S.workspaces.map((w, i) => ({ label: `${w.name}  ·  ${counts[w.id] || 0} tabs`, checked: w.id === (S.workspace || ''), kbd: i < 9 ? `Ctrl+Shift+${i + 1}` : '', run: () => call('workspaces.switch', { id: w.id }) })),
    '-',
    { label: 'New workspace…', icon: 'plus', run: () => newWorkspace([]) },
    { label: 'Manage workspaces', icon: 'gear', run: () => call('openInternal', { page: 'settings/#workspaces' }) },
  ], { anchor: $('#ws-btn') });
}
function newWorkspace(ids) {
  const el = h(`<div class="popup bubble"><h3>New workspace</h3><label>Name</label><input class="field" placeholder="e.g. School, Research" maxlength="30"><label>Colour</label><div class="swatches">${GROUP_COLORS.map((c, i) => `<button class="swatch${i === 0 ? ' on' : ''}" data-c="${c}" style="background:${gcolor(c)}"></button>`).join('')}</div><div class="actions"><button class="btn" data-x>Cancel</button><button class="btn primary" data-ok>Create</button></div></div>`);
  let color = GROUP_COLORS[0];
  $$('.swatch', el).forEach((b) => b.addEventListener('click', () => { color = b.dataset.c; $$('.swatch', el).forEach((x) => x.classList.toggle('on', x === b)); }));
  const create = () => {
    const name = $('input', el).value.trim(); if (!name) return;
    const css = getComputedStyle(document.documentElement).getPropertyValue(`--g-${color}`).trim();
    call('workspaces.create', { name, color: css, moveTabs: ids }).then((id) => { closePopups(); if (!ids.length) call('workspaces.switch', { id }); });
  };
  $('[data-ok]', el).onclick = create;
  $('[data-x]', el).onclick = () => closePopups();
  $('input', el).addEventListener('keydown', (e) => { if (e.key === 'Enter') create(); });
  openPopup(el, { anchor: $('#ws-btn') });
}

// ------------------------------------------------------------------ toolbar + omnibox
function displayUrl(u) {
  if (!u) return '';
  if (u.startsWith('solus://newtab')) return '';
  try {
    const x = new URL(u);
    if (x.protocol === 'https:') return decodeURI(x.host + x.pathname + x.search + x.hash).replace(/\/$/, '');
    if (x.protocol === 'solus:') return 'solus://' + x.host + (x.pathname !== '/' ? x.pathname : '') + x.hash;
    return decodeURI(u);
  } catch { return u; }
}

function renderToolbar() {
  const t = activeTab();
  $('#back-btn').disabled = !t || !t.canBack;
  $('#fwd-btn').disabled = !t || !t.canFwd;
  $('#reload-btn').innerHTML = icon(t && t.loading ? 'stop' : 'reload', t && t.loading ? 18 : 17);
  $('#reload-btn').title = t && t.loading ? 'Stop loading' : `Reload (${kbd('reload')})`;
  $('#load-bar').classList.toggle('on', !!(t && t.loading));
  if (!Omni.focused) $('#omni-input').value = t ? displayUrl(t.url) : '';
  const chip = $('#site-chip');
  const sec = t ? t.security : 'other';
  chip.className = 'chip ' + sec;
  chip.innerHTML = sec === 'secure' ? icon('lock', 15)
    : sec === 'insecure' ? icon('warn', 15) + '<span>Not secure</span>'
    : sec === 'danger' ? icon('warn', 15) + '<span>Dangerous</span>'
    : sec === 'internal' ? `<img src="/img/logo-256.png" width="16" height="16" alt="">`
    : sec === 'file' ? icon('folder', 15) : icon('info', 15);
  chip.hidden = !t || (t.url || '').startsWith('solus://newtab');
  const info = (t && S.pageInfo[t.id]) || {};
  const web = t && /^https?:/.test(t.url);
  const star = $('#star-btn');
  star.hidden = !t || !(web || (t.url.startsWith('solus://') && !t.url.startsWith('solus://newtab')));
  star.classList.toggle('on', !!(info.bookmark && info.url === t?.url));
  star.innerHTML = icon(info.bookmark && info.url === t?.url ? 'starFill' : 'star', 17);
  $('#notes-btn').hidden = !web;
  $('#notes-btn').classList.toggle('on', !!info.notes);
  $('#key-btn').hidden = !(web && info.passwords > 0);
  $('#translate-btn').hidden = !(web && t && S.translateOffer && S.translateOffer.tab === t.id);
  const z = $('#zoom-chip');
  z.hidden = !t || Math.abs((t.zoom || 1) - 1) < 0.01;
  if (t) z.textContent = Math.round((t.zoom || 1) * 100) + '%';
  renderShield();
}

function renderShield() {
  const t = activeTab();
  const n = t ? (S.blocked[t.id] ?? t.blocked ?? 0) : 0;
  const b = $('#shield-btn');
  b.innerHTML = icon(S.settings.blocking_level === 'off' ? 'shield' : 'shieldCheck', 18) + (n > 0 ? `<span class="badge">${n > 99 ? '99+' : n}</span>` : '');
  b.title = S.settings.blocking_level === 'off' ? 'Ad and tracker blocking is off' : `${n} ads and trackers blocked on this page`;
}

$('#back-btn').addEventListener('click', () => call('tabAction', { action: 'back' }));
$('#fwd-btn').addEventListener('click', () => call('tabAction', { action: 'forward' }));
$('#reload-btn').addEventListener('click', (e) => { const t = activeTab(); call('tabAction', { action: t && t.loading ? 'stop' : (e.shiftKey || e.ctrlKey ? 'hardReload' : 'reload') }); });
$('#home-btn').addEventListener('click', () => call('action', { name: 'home' }));
$('#zoom-chip').addEventListener('click', () => call('tabAction', { action: 'zoomReset' }));

const Omni = {
  focused: false, items: [], sel: 0, seq: 0, lastQ: '', userText: '', noInline: false,
  input: $('#omni-input'), drop: null,
  focus() {
    this.input.focus();
    const t = activeTab();
    if (t && !t.url.startsWith('solus://newtab')) this.input.value = t.url;
    this.input.select();
  },
  open() {
    if (this.drop && this.drop.isConnected) return;
    const box = $('#omnibox');
    this.drop = h('<div class="popup omni-drop" role="listbox"></div>');
    box.classList.add('open');
    openPopup(this.drop, { anchor: box, gap: 0, focus: false, owner: box, onClose: () => { box.classList.remove('open'); this.drop = null; } });
    box.dataset.popupOwner = '1';
    this.position();
  },
  position() {
    if (!this.drop) return;
    const r = $('#omnibox').getBoundingClientRect();
    this.drop.style.left = r.left + 'px'; this.drop.style.top = r.bottom + 'px'; this.drop.style.width = r.width + 'px';
  },
  close() { if (this.drop) closePopup(this.drop); },
  maybePrerender(it) {
    clearTimeout(this.preT);
    if (!it || it.type !== 'history' || !/^https:/.test(it.url)) return;
    this.preT = setTimeout(() => { if (this.focused) call('perf.prerender', { url: it.url }).catch(() => {}); }, 300);
  },
  query() {
    const q = this.input.value.slice(0, this.input.selectionStart === this.input.selectionEnd ? undefined : this.input.selectionStart);
    const text = this.userText;
    if (!text.trim()) { this.items = []; this.close(); return; }
    const seq = ++this.seq;
    this.lastQ = text;
    call('suggest', { q: text }).then((r) => { if (seq === this.seq) this.show(r.items || [], r.inline); }).catch(() => {});
    void q;
  },
  more(d) { if (d.q === this.lastQ && this.focused) this.show(d.items, null, true); },
  show(items, inline, keepSel) {
    this.items = items;
    if (!keepSel) this.sel = 0;
    if (!items.length || !this.focused) { this.close(); return; }
    this.open();
    this.drop.innerHTML = items.map((it, i) => {
      const ic = it.type === 'search' || it.type === 'suggest' ? icon('search', 16) : it.type === 'history' ? icon('history', 16) : it.type === 'bookmark' ? icon('star', 16) : it.type === 'tab' ? (it.icon ? `<img src="${esc(it.icon)}">` : icon('tabs', 16)) : icon('globe', 16);
      const main = it.type === 'search' || it.type === 'suggest' ? `<span class="main">${esc(it.text)}</span><span class="kind">— ${it.type === 'search' ? 'Search' : ''}</span>`
        : `<span class="main">${esc(it.text || it.url)}</span><span class="url">${it.text ? '— ' + esc(displayUrl(it.url)) : ''}</span>`;
      return `<div class="sug${i === this.sel ? ' sel' : ''}" data-i="${i}"><span class="ic-slot">${ic}</span>${main}${it.type === 'tab' ? '<span class="sw">Switch to tab</span>' : ''}</div>`;
    }).join('');
    // inline autocomplete only while typing forwards
    if (inline && !this.noInline && this.input.value === this.userText && inline.toLowerCase().startsWith(this.userText.toLowerCase())) {
      this.input.value = this.userText + inline.slice(this.userText.length);
      this.input.setSelectionRange(this.userText.length, this.input.value.length);
      const hit = items.find((x) => x.type === 'history' && x.url.replace(/^https?:\/\/(www\.)?/, '').toLowerCase().startsWith(inline.toLowerCase()));
      if (hit) this.maybePrerender(hit);
    }
    reportLayout();
  },
  setSel(i) {
    this.sel = (i + this.items.length) % this.items.length;
    $$('.sug', this.drop).forEach((el, j) => el.classList.toggle('sel', j === this.sel));
    const it = this.items[this.sel];
    if (it) { this.input.value = it.type === 'search' || it.type === 'suggest' ? it.text : it.url; }
    this.maybePrerender(it);
  },
  go(i, newTab, background) {
    const it = i != null ? this.items[i] : null;
    const text = this.input.value.trim();
    this.close();
    if (it && it.type === 'tab') { call('activate', { id: it.tab }); this.input.blur(); return; }
    const input = it && i !== 0 ? it.url : text;
    if (!input) return;
    call('navigate', { input, newTab: !!newTab, background: !!background });
    this.input.blur();
  },
};
Omni.input.addEventListener('focus', () => { Omni.focused = true; $('#omnibox').classList.add('focused'); });
Omni.input.addEventListener('blur', () => {
  Omni.focused = false; $('#omnibox').classList.remove('focused');
  setTimeout(() => { if (!Omni.focused) { Omni.close(); renderToolbar(); } }, 120);
});
Omni.input.addEventListener('mousedown', (e) => { if (!Omni.focused) { e.preventDefault(); Omni.focus(); } });
Omni.input.addEventListener('input', (e) => {
  Omni.noInline = e.inputType && e.inputType.startsWith('delete');
  Omni.userText = Omni.input.value;
  clearTimeout(Omni.t); Omni.t = setTimeout(() => Omni.query(), 40);
});
Omni.input.addEventListener('keydown', (e) => {
  if (e.key === 'ArrowDown' && Omni.items.length) { e.preventDefault(); Omni.setSel(Omni.sel + 1); }
  else if (e.key === 'ArrowUp' && Omni.items.length) { e.preventDefault(); Omni.setSel(Omni.sel - 1); }
  else if (e.key === 'Enter') {
    e.preventDefault();
    let i = Omni.items.length && Omni.drop ? Omni.sel : null;
    if (i === 0 && Omni.items[0] && Omni.input.value !== Omni.userText) i = null; // inline completion accepted
    if (e.ctrlKey && !e.altKey && /^[a-z0-9-]+$/i.test(Omni.input.value.trim())) { Omni.input.value = 'www.' + Omni.input.value.trim() + '.com'; i = null; }
    Omni.go(i, e.altKey || e.shiftKey, false);
  } else if (e.key === 'Escape') {
    e.preventDefault();
    if (Omni.drop) Omni.close(); else { Omni.input.blur(); call('focusContent'); }
  } else if (e.key === 'Delete' && e.shiftKey && Omni.items[Omni.sel]?.type === 'history') {
    call('history.deleteUrl', { url: Omni.items[Omni.sel].url }).then(() => Omni.query());
  }
});
$('#omnibox').addEventListener('click', (e) => {
  const s = e.target.closest('.sug');
  if (s) Omni.go(+s.dataset.i, e.ctrlKey, e.ctrlKey);
});
document.addEventListener('click', (e) => { const s = e.target.closest('.omni-drop .sug'); if (s) Omni.go(+s.dataset.i, e.ctrlKey, e.ctrlKey); });
document.addEventListener('auxclick', (e) => { const s = e.target.closest('.omni-drop .sug'); if (s && e.button === 1) Omni.go(+s.dataset.i, true, true); });
function focusOmnibox(text) {
  Omni.focus();
  if (text != null) { Omni.input.value = text; Omni.userText = text; Omni.query(); }
}

$('#star-btn').addEventListener('click', () => bookmarkBubble());
$('#site-chip').addEventListener('click', () => siteInfo());
$('#notes-btn').addEventListener('click', () => window.Panels && Panels.open('notes'));
$('#key-btn').addEventListener('click', () => passwordsBubble());
$('#translate-btn').addEventListener('click', () => translateMenu());
$('#shield-btn').addEventListener('click', () => shieldBubble());
$('#ai-btn').addEventListener('click', () => window.AI && AI.toggle());
$('#sidebar-btn').addEventListener('click', () => window.Panels && Panels.toggle());
$('#menu-btn').addEventListener('click', () => appMenu());
$('#profile-btn').addEventListener('click', () => profileMenu());
$('#dl-btn').addEventListener('click', () => window.Panels && Panels.open('downloads'));
$('#ext-btn').addEventListener('click', () => extensionsMenu());

function renderProfile() {
  const p = S.profile || {};
  const b = $('#profile-btn');
  if (S.incognito) { b.innerHTML = `<span class="avatar" style="background:#5f3dc4">${icon('incognito', 15)}</span>`; b.title = 'Incognito'; return; }
  b.innerHTML = `<span class="avatar" style="background:${esc(p.color || '#e8590c')}">${esc((p.name || 'P').slice(0, 1).toUpperCase())}</span>`;
  b.title = `Profile: ${p.name || ''}`;
}

function renderDownloadsBtn() {
  const active = S.downloads.filter((d) => d.state === 'in_progress');
  const recent = S.downloads.filter((d) => Date.now() - (d.ended || d.started) < 30 * 60000 || d.state === 'in_progress');
  const b = $('#dl-btn');
  b.hidden = !recent.length;
  if (active.length) {
    const tot = active.reduce((a, d) => a + (d.total || 0), 0), rec = active.reduce((a, d) => a + (d.received || 0), 0);
    const pct = tot ? Math.round((rec / tot) * 100) : 30;
    b.innerHTML = icon('download', 18) + `<span class="ring" style="background:conic-gradient(var(--accent) ${pct}%, transparent 0);mask:radial-gradient(circle, transparent 12px, #000 12.5px);-webkit-mask:radial-gradient(circle, transparent 12px, #000 12.5px)"></span>`;
  } else b.innerHTML = icon('download', 18);
  requestAnimationFrame(reportLayout);
}

// ------------------------------------------------------------------ bubbles
function bookmarkBubble() {
  const t = activeTab(); if (!t) return;
  const info = S.pageInfo[t.id] || {};
  const existing = info.bookmark && info.url === t.url ? info.bookmark : null;
  const go = (id) => call('bookmarks.get', { id }).then((bm) => call('bookmarks.tree', { parent: 'root', depth: 6 }).then((tree) => show(bm, tree)));
  if (existing) go(existing);
  else call('bookmarks.add', { parent: 'bar', title: t.title || t.url, url: t.url }).then(go);
  function show(bm, tree) {
    const folders = [];
    (function walk(nodes, depth) { for (const n of nodes) if (n.kind === 'folder') { folders.push({ id: n.id, title: '  '.repeat(depth) + n.title }); walk(n.children || [], depth + 1); } })(tree, 0);
    const el = h(`<div class="popup bubble"><h3>${existing ? 'Edit bookmark' : 'Bookmark added'}</h3>
      <label>Name</label><input class="field" data-n value="${esc(bm.title)}">
      <label>Folder</label><select data-f>${folders.map((f) => `<option value="${esc(f.id)}"${f.id === bm.parent ? ' selected' : ''}>${esc(f.title)}</option>`).join('')}</select>
      <div class="actions"><button class="btn danger" data-rm>Remove</button><span style="flex:1"></span><button class="btn primary" data-ok>Done</button></div></div>`);
    const save = () => {
      const name = $('[data-n]', el).value.trim();
      const f = $('[data-f]', el).value;
      const ps = [];
      if (name !== bm.title) ps.push(call('bookmarks.update', { id: bm.id, title: name }));
      if (f !== bm.parent) ps.push(call('bookmarks.move', { id: bm.id, parent: f, index: 9999 }));
      Promise.all(ps).finally(() => closePopups());
    };
    $('[data-ok]', el).onclick = save;
    $('[data-n]', el).addEventListener('keydown', (e) => { if (e.key === 'Enter') save(); });
    $('[data-rm]', el).onclick = () => { call('bookmarks.delete', { id: bm.id }); closePopups(); };
    openPopup(el, { anchor: $('#star-btn'), align: 'right' });
    $('[data-n]', el).select();
  }
}

function siteInfo() {
  const t = activeTab(); if (!t) return;
  let origin = '';
  try { origin = new URL(t.url).origin; } catch {}
  const sec = t.security;
  const head = sec === 'secure' ? `${icon('lock', 16)} Connection is secure` : sec === 'insecure' ? `${icon('warn', 16)} Connection is not secure` : sec === 'danger' ? `${icon('warn', 16)} Certificate problem — information you send could be read by others` : sec === 'internal' ? 'This is a Solus page' : 'Local or special page';
  const el = h(`<div class="popup bubble"><h3 style="display:flex;gap:8px;align-items:center">${head}</h3><div class="note">${esc(origin || t.url)}</div><div data-perms style="margin-top:10px"></div>
    <div class="actions"><button class="btn" data-clear>Clear site data</button><span style="flex:1"></span><button class="btn" data-settings>Site settings</button></div></div>`);
  $('[data-clear]', el).hidden = !/^https?:/.test(t.url);
  $('[data-settings]', el).hidden = !/^https?:/.test(t.url);
  $('[data-clear]', el).onclick = () => call('sites.clearData', { origin }).then(() => { closePopups(); toast('Site data cleared. Reload the page to see the effect.', { label: 'Reload', run: () => call('tabAction', { action: 'reload' }) }); });
  $('[data-settings]', el).onclick = () => { closePopups(); call('openInternal', { page: 'settings/#site=' + encodeURIComponent(origin) }); };
  if (/^https?:/.test(t.url)) {
    call('permissions.forOrigin', { origin }).then((perms) => {
      const box = $('[data-perms]', el);
      box.innerHTML = (perms || []).filter((p) => p.state !== 'default' || p.used).map((p) => `<div class="kv"><span>${esc(p.label)}</span><select data-k="${esc(p.kind)}"><option value="default">Ask</option><option value="allow"${p.state === 'allow' ? ' selected' : ''}>Allow</option><option value="block"${p.state === 'block' ? ' selected' : ''}>Block</option></select></div>`).join('') || '<div class="note">No permissions have been requested by this site.</div>';
      $$('select', box).forEach((s) => s.addEventListener('change', () => call('permissions.set', { origin, kind: s.dataset.k, state: s.value })));
      reportLayout();
    }).catch(() => {});
  }
  openPopup(el, { anchor: $('#site-chip') });
}

function shieldBubble() {
  const t = activeTab();
  let site = '';
  try { site = new URL(t.url).hostname; } catch {}
  const web = t && /^https?:/.test(t.url);
  const level = S.settings.blocking_level;
  const exempt = (S.settings.site_exceptions || []).includes(site);
  const n = t ? (S.blocked[t.id] ?? t.blocked ?? 0) : 0;
  const el = h(`<div class="popup bubble"><h3>Privacy protection</h3>
    <div class="kv"><span>Ads &amp; trackers blocked here</span><b>${n}</b></div>
    <div class="kv"><span>Blocking level</span><select data-l><option value="off">Off</option><option value="balanced">Balanced</option><option value="strict">Strict</option></select></div>
    ${web ? `<div class="kv"><span>Protection on ${esc(site)}</span><select data-site><option value="on">On</option><option value="off"${exempt ? ' selected' : ''}>Off for this site</option></select></div>` : ''}
    <div class="note" style="margin-top:8px">Balanced blocks ads and trackers while avoiding most site breakage. Strict blocks more and may break some sites.</div>
    <div class="actions"><button class="btn" data-dash>Privacy dashboard</button></div></div>`);
  $('[data-l]', el).value = level;
  $('[data-l]', el).onchange = (e) => call('setSetting', { key: 'blocking_level', value: e.target.value }).then(() => toast('Reload pages to apply the new blocking level.'));
  const ss = $('[data-site]', el);
  if (ss) ss.onchange = () => call('privacy.siteException', { site, exempt: ss.value === 'off' }).then(() => call('tabAction', { action: 'reload' }));
  $('[data-dash]', el).onclick = () => { closePopups(); call('action', { name: 'privacy' }); };
  openPopup(el, { anchor: $('#shield-btn'), align: 'right' });
}

function passwordsBubble() {
  const t = activeTab(); if (!t) return;
  call('passwords.forPage', { tab: t.id }).then((list) => {
    showMenu([{ head: 'Fill a saved password' }, ...list.map((p) => ({ label: p.username || '(no username)', icon: 'key', run: () => call('passwords.fill', { tab: t.id, id: p.id }) })), '-', { label: 'Manage passwords', icon: 'gear', run: () => call('openInternal', { page: 'passwords' }) }], { anchor: $('#key-btn'), align: 'right' });
  });
}

function translateMenu() {
  const t = activeTab(); const o = S.translateOffer; if (!t || !o) return;
  showMenu([
    { head: `Page language: ${o.langName || o.lang}` },
    { label: `Translate to ${o.targetName || 'your language'}`, icon: 'translate', run: () => call('translate.page', { tab: t.id }) },
    { label: `Always translate ${o.langName || o.lang}`, run: () => call('translate.always', { lang: o.lang, tab: t.id }) },
    { label: `Never translate ${o.langName || o.lang}`, run: () => { call('translate.never', { lang: o.lang }); S.translateOffer = null; renderToolbar(); } },
  ], { anchor: $('#translate-btn'), align: 'right' });
}
solus.on('translateOffer', (d) => {
  S.translateOffer = d;
  renderToolbar();
  if (d.tab === S.active && d.prompt) infobar('tr-' + d.tab, { icon: 'translate', html: `This page is in <b>${esc(d.langName || d.lang)}</b>. Translate it to ${esc(d.targetName)}?`,
    buttons: [{ label: 'Translate', primary: true, run: () => call('translate.page', { tab: d.tab }) }, { label: 'Never for this language', run: () => call('translate.never', { lang: d.lang }) }] });
});

function extensionsMenu() {
  call('ext.list').then((list) => {
    showMenu([
      { head: 'Extensions' },
      ...list.filter((x) => x.enabled).map((x) => ({ label: x.name, img: x.icon || null, icon: x.icon ? null : 'puzzle', sub: [
        x.popup ? { label: 'Open', icon: 'external', run: () => call('ext.openPopup', { id: x.id }) } : null,
        x.options ? { label: 'Options', icon: 'gear', run: () => call('ext.openOptions', { id: x.id }) } : null,
        { label: 'Manage', icon: 'puzzle', run: () => call('openInternal', { page: 'extensions' }) },
      ].filter(Boolean) })),
      list.length ? '-' : { label: 'No extensions installed', disabled: true },
      { label: 'Manage extensions', icon: 'gear', run: () => call('openInternal', { page: 'extensions' }) },
    ], { anchor: $('#ext-btn'), align: 'right' });
  });
}

// ------------------------------------------------------------------ menus
function appMenu() {
  const t = activeTab();
  const z = Math.round(((t && t.zoom) || 1) * 100);
  const zoomRow = h(`<div class="zoom-row"><span class="lbl">Zoom</span><button data-z="out" title="Zoom out">${icon('minus', 14)}</button><span class="z">${z}%</span><button data-z="in" title="Zoom in">${icon('plus', 14)}</button><button data-z="fs" title="Full screen (F11)">${icon('maximize', 14)}</button></div>`);
  zoomRow.addEventListener('click', (e) => {
    const b = e.target.closest('[data-z]'); if (!b) return;
    if (b.dataset.z === 'fs') { closePopups(); call('action', { name: 'fullscreen' }); return; }
    call('tabAction', { action: b.dataset.z === 'in' ? 'zoomIn' : 'zoomOut' }).then(() => setTimeout(() => { $('.z', zoomRow).textContent = Math.round(((activeTab() || {}).zoom || 1) * 100) + '%'; }, 60));
  });
  const perf = { performance: 'Performance', balanced: 'Balanced', efficiency: 'Efficiency' };
  showMenu([
    { label: 'New tab', icon: 'plus', kbd: kbd('newTab'), run: () => call('newTab', {}) },
    { label: 'New window', icon: 'tabs', kbd: kbd('newWindow'), run: () => call('action', { name: 'newWindow' }) },
    { label: 'New Incognito window', icon: 'incognito', kbd: kbd('newIncognito'), run: () => call('action', { name: 'newIncognito' }) },
    '-',
    { label: 'History', icon: 'history', kbd: kbd('history'), sub: () => call('history.query', { limit: 8 }).then((list) => [
      { label: 'Show full history', icon: 'history', kbd: kbd('history'), run: () => call('action', { name: 'history' }) },
      ...(S.closedCount ? [{ label: 'Recently closed', icon: 'reload', sub: () => call('closedTabs').then((c) => c.slice(0, 10).map((x, i) => ({ label: x.title || x.url, img: x.favicon || null, icon: x.favicon ? null : 'globe', run: () => call('reopenClosedAt', { index: i }) }))) }] : []),
      '-', ...list.map((x) => ({ label: x.title || x.url, img: x.icon || null, icon: x.icon ? null : 'globe', run: () => call('navigate', { input: x.url }) })),
    ]) },
    { label: 'Downloads', icon: 'download', kbd: kbd('downloads'), run: () => window.Panels && Panels.open('downloads') },
    { label: 'Bookmarks', icon: 'star', kbd: kbd('bookmarksManager'), run: () => call('action', { name: 'bookmarksManager' }) },
    { label: 'Notes & tasks', icon: 'note', run: () => window.Panels && Panels.open('notes') },
    { label: 'Passwords', icon: 'key', run: () => call('openInternal', { page: 'passwords' }) },
    { label: 'Extensions', icon: 'puzzle', run: () => call('openInternal', { page: 'extensions' }) },
    '-',
    { el: zoomRow },
    '-',
    { label: 'Find…', icon: 'search', kbd: kbd('find'), run: () => openFind() },
    { label: 'Print…', icon: 'print', kbd: 'Ctrl+P', run: () => call('tabAction', { action: 'print' }) },
    { label: 'Save page as…', icon: 'save', kbd: 'Ctrl+S', run: () => call('tabAction', { action: 'saveAs' }) },
    { label: S.split ? 'Exit split screen' : 'Split screen', icon: 'split', kbd: kbd('splitView'), run: () => call(S.split ? 'unsplit' : 'split', {}) },
    { label: 'Picture-in-picture', icon: 'pip', run: () => call('perf.pip', {}) },
    { label: 'Translate page', icon: 'translate', disabled: !(t && /^https?:/.test(t.url)), run: () => call('translate.page', { tab: t.id, force: true }) },
    '-',
    { label: 'More tools', icon: 'tools', sub: [
      { label: 'Task manager', icon: 'gauge', kbd: kbd('taskManager'), run: () => call('action', { name: 'taskManager' }) },
      { label: 'Privacy & security dashboard', icon: 'shield', run: () => call('action', { name: 'privacy' }) },
      { label: 'Theme editor', icon: 'palette', run: () => call('openInternal', { page: 'themes' }) },
      { label: 'Import from Chrome', icon: 'import', run: () => call('openInternal', { page: 'settings/#import' }) },
      { label: 'Clear browsing data…', icon: 'trash', kbd: kbd('clearData'), run: () => call('action', { name: 'clearData' }) },
      '-',
      { label: 'Developer tools', icon: 'code', kbd: 'F12', run: () => call('tabAction', { action: 'devtools' }) },
    ] },
    { label: `Performance: ${perf[S.perf] || 'Balanced'}`, icon: 'bolt', sub: () => Object.entries(perf).map(([k, v]) => ({ label: v, checked: S.perf === k, run: () => call('app.setLocal', { key: 'perf_profile', value: k }).then(() => { S.perf = k; }) })) },
    '-',
    { label: 'Settings', icon: 'gear', kbd: kbd('settings'), run: () => call('action', { name: 'settings' }) },
    { label: 'About Solus', icon: 'info', run: () => call('openInternal', { page: 'settings/#about' }) },
    { label: 'Exit Solus', icon: 'exit', run: () => call('app.quit') },
  ], { anchor: $('#menu-btn'), align: 'right' });
}

function profileMenu() {
  if (S.incognito) {
    showMenu([{ head: 'Incognito' }, { label: 'Pages you view here are not saved to history, and cookies are deleted when you close all Incognito windows.', disabled: true }, '-', { label: 'Close Incognito window', icon: 'close', run: () => call('window', { action: 'close' }) }], { anchor: $('#profile-btn'), align: 'right' });
    return;
  }
  call('profiles.list').then((d) => {
    const others = d.profiles.filter((p) => p.id !== S.profile.id);
    showMenu([
      { head: S.profile.name || 'Profile' },
      ...others.map((p) => ({ label: p.name, icon: 'user', run: () => call('profiles.open', { id: p.id }) })),
      others.length ? '-' : null,
      { label: 'Add profile…', icon: 'plus', run: () => addProfile() },
      { label: 'Manage profiles', icon: 'gear', run: () => call('openInternal', { page: 'settings/#profiles' }) },
      { label: 'Sync & backups', icon: 'cloud', run: () => call('openInternal', { page: 'settings/#sync' }) },
      '-',
      { label: 'Open Incognito window', icon: 'incognito', run: () => call('action', { name: 'newIncognito' }) },
    ], { anchor: $('#profile-btn'), align: 'right' });
  });
}
function addProfile() {
  const el = h(`<div class="popup bubble"><h3>Add profile</h3><label>Name</label><input class="field" placeholder="e.g. Work" maxlength="40"><div class="note" style="margin-top:8px">Each profile has its own bookmarks, history, passwords, cookies and settings.</div><div class="actions"><button class="btn" data-x>Cancel</button><button class="btn primary" data-ok>Create</button></div></div>`);
  const ok = () => { const n = $('input', el).value.trim(); if (n) call('profiles.create', { name: n, color: ['#1c7ed6', '#2f9e44', '#7048e8', '#d6336c', '#0c8599'][S.profiles.length % 5] }).then(() => closePopups()).catch((e) => toast(e.message)); };
  $('[data-ok]', el).onclick = ok;
  $('[data-x]', el).onclick = () => closePopups();
  $('input', el).addEventListener('keydown', (e) => { if (e.key === 'Enter') ok(); });
  openPopup(el, { anchor: $('#profile-btn'), align: 'right' });
}

// ------------------------------------------------------------------ bookmarks bar
function bmIcon(b) {
  if (b.kind === 'folder') return icon('folder', 16);
  if (b.icon) return `<img src="${esc(b.icon)}" alt="">`;
  if ((b.url || '').startsWith('solus://')) return '<img src="/img/logo-256.png" alt="">';
  return icon('globe', 16);
}
function openBookmark(b, e) {
  const newTab = e && (e.ctrlKey || e.button === 1 || e.metaKey);
  if (newTab) call('newTab', { url: b.url, background: !e.shiftKey });
  else if (e && e.shiftKey) call('action', { name: 'newWindow' });
  else call('navigate', { input: b.url });
}
function folderItems(id) {
  return call('bookmarks.children', { parent: id }).then((kids) => {
    const items = kids.map((k) => k.kind === 'folder'
      ? { label: k.title || 'Folder', icon: 'folder', sub: () => folderItems(k.id), context: (e) => bmContext(k, e) }
      : { label: k.title || k.url, img: k.icon || null, icon: k.icon ? null : 'globe', title: k.url, run: () => openBookmark(k), middle: () => call('newTab', { url: k.url, background: true }), context: (e) => bmContext(k, e) });
    if (!items.length) return [{ label: '(empty)', disabled: true }];
    const urls = kids.filter((k) => k.kind === 'url');
    if (urls.length > 1) items.push('-', { label: `Open all (${urls.length})`, icon: 'tabs', run: () => urls.forEach((u, i) => call('newTab', { url: u.url, background: i > 0 })) });
    return items;
  });
}
function loadBookmarksBar() {
  call('bookmarks.children', { parent: 'bar' }).then((kids) => {
    const box = $('#bm-items');
    box.innerHTML = kids.length ? '' : `<span class="bm-empty">Bookmark pages with ${icon('star', 13).replace('class="ic ', 'style="display:inline;vertical-align:-2px" class="ic ')} — they appear here.</span>`;
    for (const k of kids) {
      const el = h(`<button class="bm" draggable="true" title="${esc(k.kind === 'folder' ? k.title : `${k.title}\n${k.url}`)}">${bmIcon(k)}<span>${esc(k.title || (k.kind === 'folder' ? 'Folder' : k.url))}</span></button>`);
      el.dataset.id = k.id;
      el.addEventListener('click', (e) => { if (k.kind === 'folder') folderItems(k.id).then((items) => showMenu(items, { anchor: el })); else openBookmark(k, e); });
      el.addEventListener('auxclick', (e) => { if (e.button === 1 && k.kind === 'url') call('newTab', { url: k.url, background: true }); });
      el.addEventListener('contextmenu', (e) => { e.preventDefault(); bmContext(k, e); });
      el.addEventListener('dragstart', (e) => { e.dataTransfer.setData('text/x-solus-bm', k.id); e.dataTransfer.effectAllowed = 'move'; });
      el.addEventListener('dragover', (e) => { e.preventDefault(); el.classList.add(k.kind === 'folder' ? 'drop-into' : 'drop-before'); });
      el.addEventListener('dragleave', () => el.classList.remove('drop-into', 'drop-before'));
      el.addEventListener('drop', (e) => {
        e.preventDefault(); el.classList.remove('drop-into', 'drop-before');
        const id = e.dataTransfer.getData('text/x-solus-bm');
        if (!id || id === k.id) return;
        if (k.kind === 'folder') call('bookmarks.move', { id, parent: k.id, index: 9999 });
        else call('bookmarks.move', { id, parent: 'bar', index: kids.findIndex((x) => x.id === k.id) });
      });
      box.appendChild(el);
    }
    // overflow menu
    requestAnimationFrame(() => {
      const items = $$('.bm', box);
      const limit = box.getBoundingClientRect().right;
      const hidden = items.filter((x) => x.getBoundingClientRect().right > limit + 1);
      hidden.forEach((x) => (x.style.visibility = 'hidden'));
      if (hidden.length) {
        const more = h(`<button class="bm" title="More bookmarks">${icon('chevRight', 14)}</button>`);
        more.style.position = 'absolute';
        box.style.position = 'relative';
        more.style.right = '0';
        more.onclick = () => showMenu(hidden.map((x) => { const k = kids.find((b) => b.id === x.dataset.id); return k.kind === 'folder' ? { label: k.title, icon: 'folder', sub: () => folderItems(k.id) } : { label: k.title || k.url, img: k.icon || null, icon: k.icon ? null : 'globe', run: () => openBookmark(k) }; }), { anchor: more, align: 'right' });
        box.appendChild(more);
      }
    });
  }).catch(() => {});
}
$('#bm-other').addEventListener('click', () => folderItems('other').then((items) => showMenu(items, { anchor: $('#bm-other'), align: 'right' })));
$('#bookmarks-bar').addEventListener('contextmenu', (e) => {
  if (e.target.closest('.bm')) return;
  e.preventDefault();
  showMenu([
    { label: 'Add this page…', icon: 'star', run: () => bookmarkBubble() },
    { label: 'Add folder…', icon: 'folder', run: () => bmEdit({ kind: 'folder', parent: 'bar' }) },
    '-',
    { label: 'Bookmark manager', icon: 'gear', run: () => call('action', { name: 'bookmarksManager' }) },
  ], { x: e.clientX, y: e.clientY });
});
$('#bookmarks-bar').addEventListener('dragover', (e) => e.preventDefault());
$('#bookmarks-bar').addEventListener('drop', (e) => {
  if (e.target.closest('.bm')) return;
  const id = e.dataTransfer.getData('text/x-solus-bm');
  if (id) call('bookmarks.move', { id, parent: 'bar', index: 9999 });
});
function bmContext(k, e) {
  showMenu(k.kind === 'folder' ? [
    { label: 'Open all in new tabs', icon: 'tabs', run: () => call('bookmarks.openAll', { id: k.id }).then((urls) => urls.forEach((u, i) => call('newTab', { url: u, background: i > 0 }))) },
    '-',
    { label: 'Rename…', icon: 'edit', run: () => bmEdit(k) },
    { label: 'Add folder…', icon: 'folder', run: () => bmEdit({ kind: 'folder', parent: k.id }) },
    { label: 'Delete', icon: 'trash', run: () => call('bookmarks.delete', { id: k.id }).then(() => toast(`Deleted "${k.title}"`)) },
  ] : [
    { label: 'Open in new tab', icon: 'plus', run: () => call('newTab', { url: k.url, background: true }) },
    { label: 'Open in new window', icon: 'tabs', run: () => call('action', { name: 'newWindow' }) },
    { label: 'Open in split view', icon: 'split', run: () => call('newTab', { url: k.url, background: true }).then((id) => call('split', { with: id })) },
    '-',
    { label: 'Edit…', icon: 'edit', run: () => bmEdit(k) },
    { label: 'Copy link', icon: 'copy', run: () => navigator.clipboard.writeText(k.url) },
    { label: 'Delete', icon: 'trash', run: () => call('bookmarks.delete', { id: k.id }) },
  ], { x: e.clientX, y: e.clientY, keep: !!e.target.closest('.menu') });
}
function bmEdit(k) {
  const isNew = !k.id;
  const el = h(`<div class="popup bubble"><h3>${isNew ? 'New folder' : k.kind === 'folder' ? 'Rename folder' : 'Edit bookmark'}</h3><label>Name</label><input class="field" data-n value="${esc(k.title || '')}">${k.kind === 'url' ? `<label>URL</label><input class="field" data-u value="${esc(k.url)}">` : ''}<div class="actions"><button class="btn" data-x>Cancel</button><button class="btn primary" data-ok>Save</button></div></div>`);
  const ok = () => {
    const title = $('[data-n]', el).value.trim();
    const url = $('[data-u]', el)?.value.trim();
    const p = isNew ? call('bookmarks.add', { parent: k.parent, kind: 'folder', title: title || 'New folder' }) : call('bookmarks.update', { id: k.id, title, url: k.kind === 'url' ? url : undefined });
    p.then(() => closePopups()).catch((e) => toast(e.message));
  };
  $('[data-ok]', el).onclick = ok;
  $('[data-x]', el).onclick = () => closePopups();
  el.addEventListener('keydown', (e) => { if (e.key === 'Enter') ok(); });
  openPopup(el, { x: innerWidth / 2 - 170, y: 120 });
}

// ------------------------------------------------------------------ find bar
function openFind() {
  S.findOpen = true;
  $('#findbar').hidden = false;
  const i = $('#find-input');
  i.focus(); i.select();
  if (i.value) findRun();
  requestAnimationFrame(reportLayout);
}
function closeFind() {
  S.findOpen = false;
  $('#findbar').hidden = true;
  call('find', { action: 'stop' });
  $('#find-count').textContent = '';
  requestAnimationFrame(reportLayout);
  call('focusContent');
}
let findCase = false;
function findRun() { call('find', { action: 'start', text: $('#find-input').value, case: findCase }); }
$('#find-input').addEventListener('input', () => { clearTimeout(S.findT); S.findT = setTimeout(findRun, 120); });
$('#find-input').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') call('find', { action: e.shiftKey ? 'prev' : 'next' });
  else if (e.key === 'Escape') closeFind();
});
$('#find-next').onclick = () => call('find', { action: 'next' });
$('#find-prev').onclick = () => call('find', { action: 'prev' });
$('#find-close').onclick = closeFind;
$('#find-case').onclick = () => { findCase = !findCase; $('#find-case').classList.toggle('on', findCase); findRun(); };

// ------------------------------------------------------------------ infobars
function infobar(id, o) {
  const box = $('#infobars');
  box.querySelector(`[data-ib="${CSS.escape(id)}"]`)?.remove();
  const el = h(`<div class="infobar" data-ib="${esc(id)}">${icon(o.icon || 'info', 18)}<div class="msg">${o.html}</div></div>`);
  for (const b of o.buttons || []) {
    const btn = h(`<button class="btn${b.primary ? ' primary' : ''}">${esc(b.label)}</button>`);
    btn.onclick = () => { el.remove(); requestAnimationFrame(reportLayout); b.run && b.run(); };
    el.appendChild(btn);
  }
  const x = h(`<button class="x-btn" title="Close">${icon('close', 15)}</button>`);
  x.onclick = () => { el.remove(); requestAnimationFrame(reportLayout); o.onClose && o.onClose(); };
  el.appendChild(x);
  el.dataset.tab = String(S.active);
  box.appendChild(el);
  // keep at most three bars
  while (box.children.length > 3) box.firstElementChild.remove();
  requestAnimationFrame(reportLayout);
  return el;
}
window.infobar = infobar;
// infobars belong to the tab they were shown for
function syncInfobars() {
  $$('#infobars .infobar').forEach((el) => { el.hidden = el.dataset.tab !== String(S.active) && !el.dataset.global; });
  requestAnimationFrame(reportLayout);
}

// ------------------------------------------------------------------ page host (crash / unloaded / split headers)
function renderPageHost() {
  syncInfobars();
  const host = $('#page-host');
  const t = activeTab();
  let html = '';
  if (S.split && tabById(S.split.a) && tabById(S.split.b) && (S.active === S.split.a || S.active === S.split.b)) {
    const W = host.clientWidth, gap = 6;
    const mid = Math.round(W * (S.splitDrag || S.split.ratio));
    const panes = [[S.split.a, 0, mid - gap / 2], [S.split.b, mid + gap / 2, W - mid - gap / 2]];
    for (const [id, x, w] of panes) {
      const pt = tabById(id);
      html += `<div class="split-head${id === S.active ? ' active' : ''}" style="left:${x}px;width:${w}px" data-pane="${id}">${pt.favicon ? `<img src="${esc(pt.favicon)}" width="14" height="14">` : icon('globe', 14)}<span class="t">${esc(pt.title)}</span><button data-swap title="Swap sides">${icon('refreshCw', 13)}</button><button data-unsplit title="Exit split view">${icon('close', 13)}</button></div>`;
      if (pt.crashed) html += crashBox(pt, x, w);
    }
    html += `<div class="split-divider" style="left:${mid - 8}px;width:16px"></div>`;
  } else if (t && t.crashed) {
    html = crashBox(t);
  } else if (t && (t.discarded || t.loading)) {
    html = `<div class="page-msg"><div class="spinner" style="width:22px;height:22px"></div></div>`;
  }
  host.innerHTML = html;
}
function crashBox(t, x, w) {
  const pos = x != null ? `style="left:${x}px;width:${w}px;right:auto;top:24px"` : '';
  return `<div class="page-msg" ${pos}><div class="box"><span class="big">${icon('crash', 48)}</span><h2>${esc(t.crashed)}</h2><p>Solus kept the rest of your tabs running. ${t.crashed.includes('responding') ? 'The page was using too many resources or stopped responding.' : 'This can happen if the page runs out of memory.'}</p><button class="btn primary" data-reload="${t.id}">Reload</button></div></div>`;
}
$('#page-host').addEventListener('click', (e) => {
  const r = e.target.closest('[data-reload]');
  if (r) { call('tabAction', { id: +r.dataset.reload, action: 'reload' }); return; }
  if (e.target.closest('[data-unsplit]')) { call('unsplit'); return; }
  if (e.target.closest('[data-swap]')) { call('swapSplit'); return; }
  const p = e.target.closest('[data-pane]');
  if (p) call('activate', { id: +p.dataset.pane });
});
$('#page-host').addEventListener('mousedown', (e) => {
  if (!e.target.closest('.split-divider')) return;
  const host = $('#page-host').getBoundingClientRect();
  const move = (ev) => { S.splitDrag = Math.min(0.85, Math.max(0.15, (ev.clientX - host.left) / host.width)); renderPageHost(); reportLayout(); };
  const up = () => { document.removeEventListener('mousemove', move); document.removeEventListener('mouseup', up); if (S.split && S.splitDrag) S.split.ratio = S.splitDrag; reportLayout(); S.splitDrag = null; };
  document.addEventListener('mousemove', move); document.addEventListener('mouseup', up);
});

// ------------------------------------------------------------------ UI actions (from shortcuts)
function runUiAction(a, d) {
  switch (a) {
    case 'focusOmnibox': focusOmnibox(); break;
    case 'search': focusOmnibox('? '.slice(0, 0)); Omni.input.select(); break;
    case 'find': openFind(); break;
    case 'findNext': S.findOpen ? call('find', { action: 'next' }) : openFind(); break;
    case 'findPrev': S.findOpen ? call('find', { action: 'prev' }) : openFind(); break;
    case 'bookmark': bookmarkBubble(); break;
    case 'tabSearch': tabSearch(); break;
    case 'sidebar': window.Panels && Panels.toggle(); break;
    case 'notes': window.Panels && Panels.open('notes'); break;
    case 'aiSidebar': window.AI && AI.toggle(); break;
    case 'aiPopup': window.AI && AI.popup(); break;
    case 'workspace': { const w = S.workspaces[d.n - 1]; if (w) call('workspaces.switch', { id: w.id }); break; }
    default: break;
  }
}

// keyboard handling inside the UI document (host handles global accelerators)
document.addEventListener('keydown', (e) => {
  if (e.key === 'F10' || (e.altKey && e.key === 'f')) { e.preventDefault(); appMenu(); }
});
document.addEventListener('contextmenu', (e) => { if (!e.target.closest('input,textarea')) e.preventDefault(); });

paintIcons();
// announce readiness only after panels.js and ai.js have run too
document.addEventListener('DOMContentLoaded', () => solus.call('ready').catch(() => {}));
window.UI = { showMenu, openPopup, closePopups, closePopup, toast, reportLayout, infobar, displayUrl, menu, bmIcon, openBookmark, bmContext, bmEdit, renderToolbar, activeTab, tabById, esc, h, $, $$, call, kbd };
