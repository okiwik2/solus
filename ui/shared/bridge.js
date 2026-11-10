// Solus bridge: request/response + events between internal pages / browser UI and the host.
(() => {
  const wv = window.chrome && window.chrome.webview;
  const pending = new Map();
  const listeners = {};
  let seq = 0;
  if (wv) {
    wv.addEventListener('message', (e) => {
      const m = e.data;
      if (m && m.re !== undefined) {
        const p = pending.get(m.re);
        if (p) {
          pending.delete(m.re);
          m.ok ? p.res(m.data) : p.rej(new Error(m.error || 'failed'));
        }
      } else if (m && m.ev) {
        (listeners[m.ev] || []).forEach((f) => { try { f(m.data); } catch (err) { console.error(err); } });
      }
    });
  }
  window.solus = {
    available: !!wv,
    call(cmd, args = {}) {
      if (!wv) return Promise.reject(new Error('bridge unavailable'));
      return new Promise((res, rej) => {
        const id = ++seq;
        pending.set(id, { res, rej });
        wv.postMessage({ id, cmd, args });
      });
    },
    send(cmd, args = {}) { if (wv) wv.postMessage({ cmd, args }); },
    on(ev, f) { (listeners[ev] ||= []).push(f); },
  };
})();
