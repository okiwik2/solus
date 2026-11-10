// Common helpers for internal pages.
'use strict';
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const h = (html) => { const t = document.createElement('template'); t.innerHTML = html.trim(); return t.content.firstElementChild; };
const call = (cmd, args) => solus.call(cmd, args);
function toast(text, ms = 3000) {
  $$('.toast').forEach((t) => t.remove());
  const t = h(`<div class="toast">${esc(text)}</div>`);
  document.body.appendChild(t);
  setTimeout(() => t.remove(), ms);
}
function modal(html, setup) {
  return new Promise((resolve) => {
    const back = h(`<div class="modal-back"><div class="modal">${html}</div></div>`);
    document.body.appendChild(back);
    const close = (v) => { back.remove(); resolve(v); };
    back.addEventListener('mousedown', (e) => { if (e.target === back) close(null); });
    back.addEventListener('keydown', (e) => { if (e.key === 'Escape') close(null); });
    setup && setup(back.firstElementChild, close);
    setTimeout(() => (back.querySelector('input,textarea,select,button.primary') || back).focus(), 0);
  });
}
function confirmBox(title, text, okLabel = 'OK', danger = false) {
  return modal(`<h3>${esc(title)}</h3><p class="muted">${esc(text)}</p><div class="actions"><button class="btn" data-x>Cancel</button><button class="btn ${danger ? 'danger' : 'primary'}" data-ok>${esc(okLabel)}</button></div>`,
    (m, close) => { $('[data-x]', m).onclick = () => close(false); $('[data-ok]', m).onclick = () => close(true); });
}
function sw(checked, attrs = '') { return `<label class="switch"><input type="checkbox" ${checked ? 'checked' : ''} ${attrs}><span></span></label>`; }
function fmtBytes(n) { n = n || 0; return n >= 1e9 ? (n / 1e9).toFixed(1) + ' GB' : n >= 1e6 ? (n / 1e6).toFixed(1) + ' MB' : n >= 1e3 ? Math.round(n / 1e3) + ' KB' : n + ' B'; }
function timeAgo(ts) {
  if (!ts) return 'never';
  const s = Math.round((Date.now() - ts) / 1000);
  if (s < 60) return 'just now';
  if (s < 3600) return Math.round(s / 60) + ' min ago';
  if (s < 86400) return Math.round(s / 3600) + ' h ago';
  return new Date(ts).toLocaleDateString();
}
// theme: internal pages follow the browser theme
(async () => {
  try { applySolusTheme(await call('getTheme')); } catch {}
  solus.on('theme', (t) => applySolusTheme(t));
})();
