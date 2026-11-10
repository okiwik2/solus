// Mock of the Solus host bridge for design previews (injected by tools/preview/server.mjs).
(() => {
  const q = new URLSearchParams(location.search);
  const dark = q.get('dark') === '1';
  const listeners = [];
  const fire = (data) => setTimeout(() => listeners.forEach((f) => f({ data })), 0);
  const themes = [{ id: 'default', name: 'Solus', builtin: true, accent: '#e8590c',
    light: { frame: '#e8ebf0', toolbar: '#ffffff', tabActive: '#ffffff', text: '#1f2328', muted: '#5f6670', field: '#eef1f5' },
    dark: { frame: '#1f2023', toolbar: '#2b2c30', tabActive: '#2b2c30', text: '#e8e9ed', muted: '#a3a7b0', field: '#1c1d20' } }];
  const theme = { dark, incognito: q.get('incog') === '1', mode: 'system', density: q.get('density') || 'comfortable', themeId: 'default', customCss: '', animations: true, themes };
  const settings = { theme: 'system', density: theme.density, search_engine: 'google', engines: [{ id: 'google', name: 'Google' }], show_home_button: false, sidebar_open: q.get('sidebar') === '1', sidebar_panel: q.get('panel') || 'bookmarks', sidebar_width: 300, blocking_level: 'balanced', site_exceptions: [], newtab: null, tab_hover_cards: true };
  const tab = (id, title, url, extra = {}) => ({ id, title, url, favicon: '', loading: false, canBack: true, canFwd: false, pinned: false, group: '', workspace: '', audible: false, muted: false, crashed: '', discarded: false, frozen: false, zoom: 1, blocked: 0, security: url.startsWith('https') ? 'secure' : url.startsWith('solus') ? 'internal' : 'insecure', loadError: '', opener: 0, lastActive: Date.now() - id * 1000, ...extra });
  const tabs = [
    tab(2, 'Solus — New Tab', 'solus://newtab/'),
    tab(3, 'Rust Programming Language', 'https://www.rust-lang.org/', { blocked: 4 }),
    tab(4, 'BBC News - Home', 'https://www.bbc.com/news', { group: 'g1', blocked: 27 }),
    tab(5, 'Weather forecast for Auckland', 'https://www.metservice.com/towns-cities/locations/auckland', { group: 'g1', discarded: true }),
    tab(6, 'YouTube', 'https://www.youtube.com/', { audible: true }),
  ];
  const active = +(q.get('active') || 3);
  const groups = [{ id: 'g1', name: 'News', color: 'blue', collapsed: false, auto: true }];
  const bm = { bar: [{ id: 'b1', kind: 'url', title: 'GitHub', url: 'https://github.com/' }, { id: 'b2', kind: 'folder', title: 'Uni' }, { id: 'b3', kind: 'url', title: 'Stuff NZ', url: 'https://www.stuff.co.nz/' }, { id: 'b4', kind: 'url', title: 'Gmail', url: 'https://mail.google.com/' }],
    root: [{ id: 'bar', kind: 'folder', title: 'Bookmarks bar' }, { id: 'other', kind: 'folder', title: 'Other bookmarks' }], other: [], b2: [{ id: 'b5', kind: 'url', title: 'Canvas', url: 'https://canvas.auckland.ac.nz/' }] };
  const now = Date.now();
  const history = [1, 2, 3, 4, 5, 6].map((i) => ({ id: i, ts: now - i * 3600e3 * 5, url: ['https://www.rust-lang.org/', 'https://www.bbc.com/news', 'https://github.com/', 'https://news.ycombinator.com/', 'https://www.stuff.co.nz/', 'https://docs.rs/'][i - 1], title: ['Rust', 'BBC News', 'GitHub', 'Hacker News', 'Stuff', 'Docs.rs'][i - 1], visits: 10 - i }));
  const handlers = {
    ready: () => { fire({ ev: 'init', data: { wid: 1, incognito: theme.incognito, version: '0.1.0', profile: { id: 'default', name: 'Omar', color: '#e8590c' }, profiles: [], settings, theme, shortcuts: [], perf: 'balanced' } }); fire({ ev: 'tabs', data: { tabs, active, groups, workspace: '', split: q.get('split') ? { a: 3, b: 4, ratio: 0.5 } : null, closedCount: 2 } }); return true; },
    getSettings: () => settings, getTheme: () => theme,
    'bookmarks.children': (a) => bm[a.parent] || [], 'bookmarks.search': () => [],
    'history.query': () => history, 'history.mostVisited': () => history,
    'workspaces.list': () => [{ id: '', name: 'Personal', color: '#e8590c' }, { id: 'w2', name: 'School', color: '#1c7ed6' }, { id: 'w3', name: 'Research', color: '#2f9e44' }],
    'downloads.list': () => [{ id: 1, url: 'https://example.com/file.zip', path: 'C:\\Users\\me\\Downloads\\report-final.pdf', state: 'in_progress', received: 3.2e6, total: 8e6, started: now - 5000 }],
    'tasks.list': () => [{ id: 't1', title: 'Finish COMPSCI assignment', due: now + 86400e3, done: false }, { id: 't2', title: 'Book dentist', due: 0, done: false }],
    'reminders.list': () => [], 'notes.list': () => [{ id: 'n1', scope: 'page', key: 'https://www.rust-lang.org/', body: 'Read the async chapter', updated: now }], 'notes.forUrl': () => [],
    'favicons.forUrls': () => ({}), suggest: (a) => ({ items: [{ type: 'search', text: a.q, url: '' }, { type: 'history', text: 'Rust Programming Language', url: 'https://www.rust-lang.org/' }, { type: 'suggest', text: a.q + ' tutorial', url: '' }], inline: null }),
    closedTabs: () => [{ url: 'https://example.com', title: 'Example', favicon: '' }],
    'themes.list': () => themes, 'app.local': () => ({ perf_profile: 'balanced', auto_battery: true, rules: { freeze_after_min: 5, discard_after_min: 45, exceptions: [] }, close_to_tray: true, tray_unload_after_min: 10, update_check: true, gpu_acceleration: true, preload: true }),
    'ai.status': () => ({ enabled: true, provider: 'anthropic', model: 'claude-opus-5-5', keySet: false, anthropicKey: false, groqKey: false, permissions: { read_page: true, read_tabs: true }, models: { anthropic: [{ id: 'claude-opus-5-5', name: 'Claude Opus 5.5', in: 4, out: 20 }], groq: [] } }),
    'sync.status': () => ({ sync: { enabled: false }, suggested: [{ name: 'OneDrive', path: 'C:\\Users\\me\\OneDrive' }], deviceName: 'LAPTOP' }),
    'perf.status': () => ({ battery: false }), 'privacy.lists': () => ({ lists: [{ name: 'easylist.txt', size: 2e6, modified: now, active: true }], engineReady: true }),
    'permissions.list': () => [], 'profiles.list': () => ({ profiles: [{ id: 'default', name: 'Omar', color: '#e8590c' }], current: 'default' }),
    'import.chromeProfiles': () => ({ installed: true, running: false, profiles: [{ dir: 'Default', name: 'Person 1', email: '' }] }),
    info: () => ({ version: '1.0.0', engine: '157.0.4341.42', dataDir: 'C:\\Users\\me\\AppData\\Local\\Solus\\User Data' }),
    'update.status': () => ({}), 'backup.list': () => [], 'translate.languages': () => ({ languages: [{ code: 'en', name: 'English' }], system: 'en' }),
    'privacy.dashboard': () => ({ blocking: { level: 'balanced', ready: true, today: 412, week: 2890, total: 5120, daily: [0, 1, 2, 3, 4, 5, 6].map((i) => ({ day: Math.floor(now / 864e5) - i, count: 200 + i * 37 })), topSites: [{ site: 'theguardian.com', count: 620 }, { site: 'nytimes.com', count: 488 }, { site: 'stuff.co.nz', count: 301 }], exceptions: [] },
      trackingPrevention: 'balanced', thirdPartyCookies: 'blocked', httpsFirst: true, smartscreen: true, permissions: [{ kind: 'notifications', label: 'Show notifications', state: 'allow', count: 2 }],
      passwords: { count: 42, weak: 3, reused: 5 }, history: { count: 1830 }, ai: { enabled: true, keySet: false, permissions: { read_history: false } }, sync: { enabled: false }, telemetry: 'none' }),
    'shortcuts.list': () => [{ action: 'newTab', keys: 'Ctrl+T', default: 'Ctrl+T', label: 'New tab' }],
  };
  window.chrome = window.chrome || {};
  window.chrome.webview = {
    addEventListener: (t, f) => { if (t === 'message') listeners.push(f); },
    postMessage: (m) => {
      const f = handlers[m.cmd];
      if (m.id === undefined) return;
      let r; try { r = f ? f(m.args || {}) : true; } catch (e) { r = null; }
      fire({ re: m.id, ok: true, data: r });
    },
  };
})();
// ?debug=1 writes UI state into the DOM for headless inspection
if (new URLSearchParams(location.search).get('debug')) {
  const ce = console.error; window.__errs = []; console.error = (...a) => { window.__errs.push(a.map((x) => (x && x.stack) || String(x)).join(' ').slice(0, 300)); ce(...a); };
  window.addEventListener('error', (e) => { document.title = 'ERR ' + e.message + ' @ ' + (e.error && e.error.stack || '').replace(/s+/g, ' ').slice(0, 400); });
  setTimeout(() => { const p = document.createElement('pre'); p.id = 'dbg'; try { p.textContent = JSON.stringify({ panels: typeof window.Panels, open: S.settings && S.settings.sidebar_open, cur: window.Panels && Panels.current(), title: document.title, errs: window.__errs, inc: S.incognito, after: (function(){ try { Panels.init(); return Panels.current(); } catch(e) { return "threw " + e.message; } })() }); } catch (e) { p.textContent = 'dbgerr ' + e.message; } document.body.appendChild(p); }, 1500);
}
