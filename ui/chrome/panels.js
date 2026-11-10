// Sidebar panels: bookmarks, history, downloads, notes, tasks & reminders, tools.
'use strict';
(() => {
  const { $, $$, h, esc, call, toast, showMenu, displayUrl } = window.UI;
  const side = $('#sidebar');
  const PANELS = [
    ['bookmarks', 'star', 'Bookmarks'], ['history', 'history', 'History'], ['downloads', 'download', 'Downloads'],
    ['notes', 'note', 'Notes'], ['tasks', 'tasks', 'Tasks & reminders'], ['tools', 'tools', 'Tools'],
  ];
  let current = null;
  let width = 320;

  function setOpen(name) {
    current = name;
    const open = !!name;
    side.hidden = !open;
    document.documentElement.style.setProperty('--sidebar-w', open ? width + 'px' : '0px');
    $('#sidebar-btn').classList.toggle('on', open);
    if (open) render();
    requestAnimationFrame(UI.reportLayout);
  }

  function shell(title) {
    side.innerHTML = `<div class="panel-tabs">${PANELS.map(([id, ic, t]) => `<button data-p="${id}" class="${id === current ? 'on' : ''}" title="${t}">${icon(ic, 17)}</button>`).join('')}<button data-close title="Close sidebar">${icon('close', 16)}</button></div>
      <div class="panel-head"><span class="title">${esc(title)}</span><span data-head-acts style="display:flex;gap:2px"></span></div><div class="panel-body"></div>
      <div data-resize style="position:absolute;top:0;bottom:0;width:6px;cursor:col-resize;left:calc(var(--sidebar-w) - 3px);z-index:3"></div>`;
    $$('.panel-tabs [data-p]', side).forEach((b) => (b.onclick = () => Panels.open(b.dataset.p)));
    $('[data-close]', side).onclick = () => Panels.close();
    const rz = $('[data-resize]', side);
    rz.onmousedown = (e) => {
      e.preventDefault();
      const move = (ev) => { width = Math.max(240, Math.min(560, ev.clientX)); document.documentElement.style.setProperty('--sidebar-w', width + 'px'); rz.style.left = (width - 3) + 'px'; UI.reportLayout(); };
      const up = () => { document.removeEventListener('mousemove', move); document.removeEventListener('mouseup', up); call('setSetting', { key: 'sidebar_width', value: width }); };
      document.addEventListener('mousemove', move); document.addEventListener('mouseup', up);
    };
    return $('.panel-body', side);
  }
  const headActs = (html) => { const a = $('[data-head-acts]', side); a.innerHTML = html; return a; };

  function render() {
    const def = PANELS.find((p) => p[0] === current);
    const body = shell(def ? def[2] : '');
    ({ bookmarks, history, downloads, notes, tasks, tools })[current]?.(body);
  }

  // ---------------------------------------------------------------- bookmarks
  function bookmarks(body) {
    headActs(`<button class="x-btn" data-add title="New folder">${icon('folder', 15)}</button><button class="x-btn" data-mgr title="Bookmark manager">${icon('external', 15)}</button>`);
    $('[data-add]', side).onclick = () => UI.bmEdit({ kind: 'folder', parent: 'other' });
    $('[data-mgr]', side).onclick = () => call('action', { name: 'bookmarksManager' });
    body.innerHTML = `<input class="panel-search" placeholder="Search bookmarks"><div data-list></div>`;
    const list = $('[data-list]', body);
    const input = $('input', body);
    const open = new Set(['bar']);
    const row = (b) => b.kind === 'folder'
      ? `<div class="row" data-folder="${esc(b.id)}">${icon(open.has(b.id) ? 'folderOpen' : 'folder', 16)}<span class="t">${esc(b.title)}</span></div><div class="tree-kids" data-kids="${esc(b.id)}"${open.has(b.id) ? '' : ' hidden'}></div>`
      : `<div class="row" data-bm="${esc(b.id)}" title="${esc(b.url)}">${UI.bmIcon(b)}<span class="t">${esc(b.title || b.url)}</span><span class="acts"><button data-del="${esc(b.id)}" title="Delete">${icon('trash', 14)}</button></span></div>`;
    const cache = {};
    function fill(el, parent) {
      call('bookmarks.children', { parent }).then((kids) => {
        kids.forEach((k) => (cache[k.id] = k));
        el.innerHTML = kids.map(row).join('') || '<div class="row sub">Empty</div>';
        $$('[data-kids]', el).forEach((k) => { if (!k.hidden) fill(k, k.dataset.kids); });
      });
    }
    function load() {
      const q = input.value.trim();
      if (q) call('bookmarks.search', { q }).then((r) => { r.forEach((k) => (cache[k.id] = k)); list.innerHTML = r.map(row).join('') || '<div class="empty">No bookmarks match</div>'; });
      else call('bookmarks.children', { parent: 'root' }).then((roots) => { roots.forEach((k) => (cache[k.id] = k)); list.innerHTML = roots.map(row).join(''); $$('[data-kids]', list).forEach((k) => { if (!k.hidden) fill(k, k.dataset.kids); }); });
    }
    input.oninput = load;
    list.onclick = (e) => {
      const del = e.target.closest('[data-del]');
      if (del) { call('bookmarks.delete', { id: del.dataset.del }); return; }
      const f = e.target.closest('[data-folder]');
      if (f) { const id = f.dataset.folder; const kids = $(`[data-kids="${CSS.escape(id)}"]`, list); if (open.has(id)) { open.delete(id); kids.hidden = true; } else { open.add(id); kids.hidden = false; fill(kids, id); } f.firstElementChild.outerHTML = icon(open.has(id) ? 'folderOpen' : 'folder', 16); return; }
      const b = e.target.closest('[data-bm]');
      if (b) UI.openBookmark(cache[b.dataset.bm], e);
    };
    list.onauxclick = (e) => { const b = e.target.closest('[data-bm]'); if (b && e.button === 1) call('newTab', { url: cache[b.dataset.bm].url, background: true }); };
    list.oncontextmenu = (e) => { e.preventDefault(); const r = e.target.closest('[data-bm],[data-folder]'); if (r) UI.bmContext(cache[r.dataset.bm || r.dataset.folder], e); };
    Panels._reload = load;
    load();
  }

  // ---------------------------------------------------------------- history
  const dayName = (ts) => {
    const d = new Date(ts), t = new Date(); t.setHours(0, 0, 0, 0);
    const diff = Math.round((t - new Date(d.getFullYear(), d.getMonth(), d.getDate())) / 86400000);
    return diff === 0 ? 'Today' : diff === 1 ? 'Yesterday' : d.toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long' });
  };
  function history(body) {
    headActs(`<button class="x-btn" data-full title="Full history page">${icon('external', 15)}</button>`);
    $('[data-full]', side).onclick = () => call('action', { name: 'history' });
    body.innerHTML = `<input class="panel-search" placeholder="Search history"><div data-list></div>`;
    const input = $('input', body), list = $('[data-list]', body);
    function load() {
      call('history.query', { text: input.value.trim(), limit: 150 }).then((rows) => {
        let html = '', last = '';
        for (const r of rows) {
          const d = dayName(r.ts);
          if (d !== last) { html += `<div class="section-label">${esc(d)}</div>`; last = d; }
          html += `<div class="row" data-url="${esc(r.url)}" title="${esc(r.url)}">${r.icon ? `<img src="${esc(r.icon)}">` : icon('globe', 16)}<div class="stack"><span class="t">${esc(r.title || r.url)}</span><span class="sub">${new Date(r.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })} · ${esc(displayUrl(r.url).split('/')[0])}</span></div><span class="acts"><button data-del="${r.id}" title="Remove from history">${icon('trash', 14)}</button></span></div>`;
        }
        list.innerHTML = html || `<div class="empty">${input.value ? 'No matches' : 'Pages you visit appear here.'}</div>`;
      });
    }
    input.oninput = () => { clearTimeout(input._t); input._t = setTimeout(load, 150); };
    list.onclick = (e) => {
      const d = e.target.closest('[data-del]');
      if (d) { call('history.deleteVisits', { ids: [+d.dataset.del] }).then(load); return; }
      const r = e.target.closest('[data-url]');
      if (r) { if (e.ctrlKey) call('newTab', { url: r.dataset.url, background: true }); else call('navigate', { input: r.dataset.url }); }
    };
    list.onauxclick = (e) => { const r = e.target.closest('[data-url]'); if (r && e.button === 1) call('newTab', { url: r.dataset.url, background: true }); };
    Panels._reload = load;
    load();
  }

  // ---------------------------------------------------------------- downloads
  const fmt = (n) => (n >= 1e9 ? (n / 1e9).toFixed(1) + ' GB' : n >= 1e6 ? (n / 1e6).toFixed(1) + ' MB' : n >= 1e3 ? Math.round(n / 1e3) + ' KB' : (n || 0) + ' B');
  function downloads(body) {
    headActs(`<button class="x-btn" data-folder title="Open downloads folder">${icon('folder', 15)}</button><button class="x-btn" data-clear title="Clear list">${icon('trash', 15)}</button>`);
    $('[data-folder]', side).onclick = () => call('downloads.openFolder');
    $('[data-clear]', side).onclick = () => call('downloads.clear');
    const list = S.downloads.slice().sort((a, b) => b.started - a.started);
    body.innerHTML = list.map((d) => {
      const name = d.path ? d.path.split(/[\\/]/).pop() : d.url;
      const pct = d.total ? Math.round((d.received / d.total) * 100) : 0;
      const status = d.state === 'in_progress' ? (d.paused ? 'Paused · ' : '') + `${fmt(d.received)}${d.total ? ' of ' + fmt(d.total) : ''}`
        : d.state === 'completed' ? fmt(d.total || d.received) + ' · ' + displayUrl(d.url).split('/')[0]
        : d.state === 'interrupted' ? 'Failed' + (d.reason ? ' — ' + d.reason : '') : 'Cancelled';
      const acts = d.state === 'in_progress'
        ? `<button data-a="${d.paused ? 'resume' : 'pause'}" data-id="${d.id}" title="${d.paused ? 'Resume' : 'Pause'}">${icon(d.paused ? 'forward' : 'minus', 14)}</button><button data-a="cancel" data-id="${d.id}" title="Cancel">${icon('close', 14)}</button>`
        : d.state === 'completed' ? `<button data-a="show" data-id="${d.id}" title="Show in folder">${icon('folder', 14)}</button>` : `<button data-a="retry" data-id="${d.id}" title="Retry">${icon('reload', 14)}</button>`;
      return `<div class="row" data-open="${d.state === 'completed' ? d.id : ''}" title="${esc(d.path || d.url)}">${icon('download', 16)}<div class="stack"><span class="t">${esc(name)}</span><span class="sub">${esc(status)}</span>${d.state === 'in_progress' ? `<div style="height:3px;border-radius:2px;background:var(--border);margin-top:4px"><div style="height:3px;border-radius:2px;background:var(--accent);width:${pct}%"></div></div>` : ''}</div><span class="acts" style="display:flex">${acts}</span></div>`;
    }).join('') || '<div class="empty">Files you download appear here.</div>';
    body.onclick = (e) => {
      const a = e.target.closest('[data-a]');
      if (a) { e.stopPropagation(); call('downloads.action', { id: +a.dataset.id, action: a.dataset.a }); return; }
      const r = e.target.closest('[data-open]');
      if (r && r.dataset.open) call('downloads.action', { id: +r.dataset.open, action: 'open' });
    };
  }

  // ---------------------------------------------------------------- notes
  function notes(body) {
    const t = UI.activeTab();
    const url = t && /^https?:/.test(t.url) ? t.url : '';
    let site = ''; try { site = url ? new URL(url).hostname.replace(/^www\./, '') : ''; } catch {}
    body.innerHTML = `${url ? `<div class="section-label">Add a note</div>
      <div style="padding:0 8px"><textarea data-body rows="3" style="width:100%" placeholder="Write a note about this page…"></textarea>
      <div style="display:flex;gap:6px;align-items:center;margin-top:6px"><select data-scope class="field" style="height:28px"><option value="page">This page</option><option value="site">All of ${esc(site)}</option></select><span style="flex:1"></span><button class="btn primary" data-save>Save note</button></div></div>
      <div class="section-label">On this page</div><div data-page></div>` : '<div class="empty">Open a web page to attach notes to it.</div>'}
      <div class="section-label">All notes</div><input class="panel-search" placeholder="Search notes"><div data-all></div>`;
    const card = (n) => `<div class="row" style="align-items:flex-start" data-note="${esc(n.id)}"><span style="width:4px;align-self:stretch;border-radius:2px;background:${n.color || 'var(--accent)'}"></span><div class="stack"><span style="white-space:pre-wrap;overflow-wrap:anywhere;user-select:text">${esc(n.body)}</span><span class="sub">${n.scope === 'site' ? 'Site: ' : ''}${esc(n.scope === 'site' ? n.key : displayUrl(n.key))} · ${new Date(n.updated).toLocaleDateString()}</span></div><span class="acts"><button data-go="${esc(n.key)}" data-scope="${n.scope}" title="Open">${icon('external', 14)}</button><button data-edit="${esc(n.id)}" title="Edit">${icon('edit', 14)}</button><button data-del="${esc(n.id)}" title="Delete">${icon('trash', 14)}</button></span></div>`;
    const notesCache = {};
    function load() {
      if (url) call('notes.forUrl', { url }).then((l) => { l.forEach((n) => (notesCache[n.id] = n)); $('[data-page]', body).innerHTML = l.map(card).join('') || '<div class="row sub">No notes yet.</div>'; });
      call('notes.list', { q: $('.panel-search', body).value }).then((l) => { l.forEach((n) => (notesCache[n.id] = n)); $('[data-all]', body).innerHTML = l.map(card).join('') || '<div class="empty">No notes</div>'; });
    }
    const save = $('[data-save]', body);
    if (save) save.onclick = () => {
      const text = $('[data-body]', body).value.trim(); if (!text) return;
      call('notes.save', { scope: $('[data-scope]', body).value, url, body: text }).then(() => { $('[data-body]', body).value = ''; load(); });
    };
    $('.panel-search', body).oninput = load;
    body.onclick = (e) => {
      const d = e.target.closest('[data-del]');
      if (d) { call('notes.delete', { id: d.dataset.del }).then(load); return; }
      const g = e.target.closest('[data-go]');
      if (g) { call('navigate', { input: g.dataset.scope === 'site' ? 'https://' + g.dataset.go : g.dataset.go, newTab: true }); return; }
      const ed = e.target.closest('[data-edit]');
      if (ed) {
        const n = notesCache[ed.dataset.edit];
        const row = ed.closest('.row');
        row.innerHTML = `<div class="stack"><textarea rows="4" style="width:100%">${esc(n.body)}</textarea><div style="display:flex;justify-content:flex-end;gap:6px;margin-top:6px"><button class="btn" data-cancel>Cancel</button><button class="btn primary" data-upd>Save</button></div></div>`;
        $('[data-cancel]', row).onclick = (ev) => { ev.stopPropagation(); load(); };
        $('[data-upd]', row).onclick = (ev) => { ev.stopPropagation(); call('notes.save', { id: n.id, scope: n.scope, url: n.scope === 'page' ? n.key : url, key: n.key, body: $('textarea', row).value }).then(load); };
      }
    };
    Panels._reload = load;
    load();
  }

  // ---------------------------------------------------------------- tasks & reminders
  function tasks(body) {
    const t = UI.activeTab();
    body.innerHTML = `<div style="padding:8px"><input class="field" data-title style="width:100%" placeholder="Add a task or reminder…">
      <div style="display:flex;gap:6px;align-items:center;margin-top:6px;flex-wrap:wrap"><input type="datetime-local" class="field" data-due style="height:28px;flex:1;min-width:150px">
      <label style="display:flex;gap:4px;align-items:center;font-size:12px"><input type="checkbox" data-remind> Remind me</label>
      <label style="display:flex;gap:4px;align-items:center;font-size:12px" title="Attach the current page"><input type="checkbox" data-link> Link page</label>
      <button class="btn primary" data-add>Add</button></div></div>
      <div class="section-label">To do</div><div data-open></div><div class="section-label">Upcoming reminders</div><div data-rem></div><div class="section-label">Done</div><div data-done></div>`;
    const when = (ts) => { const d = new Date(ts); return d.toLocaleDateString(undefined, { day: 'numeric', month: 'short' }) + ' ' + d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }); };
    const taskRow = (x) => `<div class="row" data-task="${esc(x.id)}"><input type="checkbox" data-done-t="${esc(x.id)}" ${x.done ? 'checked' : ''}><div class="stack"><span class="t" style="${x.done ? 'text-decoration:line-through;color:var(--muted)' : ''}">${esc(x.title)}</span>${x.due || x.url ? `<span class="sub" style="${x.due && x.due < Date.now() && !x.done ? 'color:var(--danger)' : ''}">${x.due ? when(x.due) : ''}${x.due && x.url ? ' · ' : ''}${x.url ? esc(displayUrl(x.url).split('/')[0]) : ''}</span>` : ''}</div><span class="acts">${x.url ? `<button data-url="${esc(x.url)}" title="Open linked page">${icon('external', 14)}</button>` : ''}<button data-del-t="${esc(x.id)}" title="Delete">${icon('trash', 14)}</button></span></div>`;
    const remRow = (r) => `<div class="row">${icon('bell', 16)}<div class="stack"><span class="t">${esc(r.title)}</span><span class="sub">${when(r.at)}</span></div><span class="acts">${r.url ? `<button data-url="${esc(r.url)}">${icon('external', 14)}</button>` : ''}<button data-del-r="${esc(r.id)}" title="Delete">${icon('trash', 14)}</button></span></div>`;
    function load() {
      call('tasks.list').then((l) => {
        $('[data-open]', body).innerHTML = l.filter((x) => !x.done).map(taskRow).join('') || '<div class="row sub">Nothing to do.</div>';
        $('[data-done]', body).innerHTML = l.filter((x) => x.done).slice(0, 30).map(taskRow).join('') || '<div class="row sub">—</div>';
      });
      call('reminders.list').then((l) => { $('[data-rem]', body).innerHTML = l.filter((r) => !r.fired).map(remRow).join('') || '<div class="row sub">No reminders.</div>'; });
    }
    $('[data-add]', body).onclick = () => {
      const title = $('[data-title]', body).value.trim(); if (!title) return;
      const dueStr = $('[data-due]', body).value;
      const due = dueStr ? new Date(dueStr).getTime() : 0;
      const url = $('[data-link]', body).checked && t && /^https?:/.test(t.url) ? t.url : '';
      const remind = $('[data-remind]', body).checked;
      if (remind && !due) { toast('Pick a date and time for the reminder.'); return; }
      const p = [call('tasks.save', { title, due, url })];
      if (remind) p.push(call('reminders.save', { title, at: due, url }));
      Promise.all(p).then(() => { $('[data-title]', body).value = ''; load(); }).catch((e) => toast(e.message));
    };
    $('[data-title]', body).addEventListener('keydown', (e) => { if (e.key === 'Enter') $('[data-add]', body).click(); });
    body.onchange = (e) => { const c = e.target.closest('[data-done-t]'); if (c) call('tasks.save', { id: c.dataset.doneT, done: c.checked }).then(load); };
    body.onclick = (e) => {
      const d = e.target.closest('[data-del-t]'); if (d) { call('tasks.delete', { id: d.dataset.delT }).then(load); return; }
      const r = e.target.closest('[data-del-r]'); if (r) { call('reminders.delete', { id: r.dataset.delR }).then(load); return; }
      const u = e.target.closest('[data-url]'); if (u) call('newTab', { url: u.dataset.url });
    };
    Panels._reload = load;
    load();
  }

  // ---------------------------------------------------------------- tools
  function tools(body) {
    const items = [
      ['gauge', 'Task manager', 'Memory and CPU used by each tab', () => call('action', { name: 'taskManager' })],
      ['shield', 'Privacy dashboard', 'Blocking, cookies, permissions, data', () => call('action', { name: 'privacy' })],
      ['key', 'Passwords', 'Saved sign-ins and Chrome import', () => call('openInternal', { page: 'passwords' })],
      ['puzzle', 'Extensions', 'Chrome Web Store extensions', () => call('openInternal', { page: 'extensions' })],
      ['palette', 'Theme editor', 'Colours, custom themes and CSS', () => call('openInternal', { page: 'themes' })],
      ['import', 'Import from Chrome', 'Bookmarks, history, passwords', () => call('openInternal', { page: 'settings/#import' })],
      ['cloud', 'Sync & backups', 'Encrypted sync and restore points', () => call('openInternal', { page: 'settings/#sync' })],
      ['grid', 'Workspaces', 'Organise tabs by context', () => call('openInternal', { page: 'settings/#workspaces' })],
      ['trash', 'Clear browsing data', 'History, cookies, cache', () => call('action', { name: 'clearData' })],
      ['code', 'Developer tools', 'Inspect the current page', () => call('tabAction', { action: 'devtools' })],
    ];
    body.innerHTML = items.map(([ic, t, sub], i) => `<div class="row" data-i="${i}" style="min-height:44px">${icon(ic, 18)}<div class="stack"><span class="t">${esc(t)}</span><span class="sub">${esc(sub)}</span></div></div>`).join('');
    body.onclick = (e) => { const r = e.target.closest('[data-i]'); if (r) items[+r.dataset.i][3](); };
  }

  window.Panels = {
    init() {
      width = S.settings.sidebar_width || 320;
      if (S.settings.sidebar_open && !S.incognito) setOpen(S.settings.sidebar_panel || 'bookmarks');
    },
    applySettings() { width = S.settings.sidebar_width || width; if (current) document.documentElement.style.setProperty('--sidebar-w', width + 'px'); },
    open(name) {
      if (current === name) { render(); return; }
      setOpen(name);
      call('setSetting', { key: 'sidebar_open', value: true }).catch(() => {});
      call('setSetting', { key: 'sidebar_panel', value: name }).catch(() => {});
    },
    close() { setOpen(null); call('setSetting', { key: 'sidebar_open', value: false }).catch(() => {}); },
    toggle() { current ? this.close() : this.open(S.settings.sidebar_panel || 'bookmarks'); },
    refresh(name) { if (current === name) { if (Panels._reload) Panels._reload(); else render(); } },
    current: () => current,
  };
  solus.on('active', () => { if (current === 'notes') render(); });
  solus.on('notesChanged', () => Panels.refresh('notes'));
  solus.on('tasksChanged', () => Panels.refresh('tasks'));
  solus.on('historyChanged', () => Panels.refresh('history'));
})();
