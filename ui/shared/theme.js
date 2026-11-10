// Applies a Solus theme (palette + light/dark + density + custom CSS) to the current document.
(() => {
  const hex = (h) => { const n = parseInt(h.slice(1), 16); return [(n >> 16) & 255, (n >> 8) & 255, n & 255]; };
  const toHex = (r, g, b) => '#' + [r, g, b].map((x) => Math.max(0, Math.min(255, Math.round(x))).toString(16).padStart(2, '0')).join('');
  const mix = (a, b, t) => { const A = hex(a), B = hex(b); return toHex(A[0] + (B[0] - A[0]) * t, A[1] + (B[1] - A[1]) * t, A[2] + (B[2] - A[2]) * t); };
  const lum = (h) => { const [r, g, b] = hex(h).map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }); return 0.2126 * r + 0.7152 * g + 0.0722 * b; };
  const contrast = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
  const onAccent = (acc) => (contrast(acc, '#ffffff') >= 3 ? '#ffffff' : '#111111');

  const INCOGNITO = { frame: '#231f2e', toolbar: '#2f2a3d', tabActive: '#2f2a3d', text: '#ece9f5', muted: '#a9a2bd', field: '#1d1a26' };
  const DENSITY = {
    compact: { tab: 34, toolbar: 38, bm: 28, font: 12.5, radius: 6, gap: 4 },
    comfortable: { tab: 40, toolbar: 44, bm: 32, font: 13, radius: 8, gap: 6 },
    spacious: { tab: 46, toolbar: 52, bm: 36, font: 14, radius: 10, gap: 8 },
  };

  window.applySolusTheme = function (t) {
    if (!t) return;
    const themes = t.themes || [];
    const theme = themes.find((x) => x.id === t.themeId) || themes.find((x) => x.id === 'default') || null;
    const dark = !!t.dark;
    let p = theme ? theme[dark ? 'dark' : 'light'] : null;
    let accent = theme ? theme.accent : '#e8590c';
    if (t.incognito) { p = INCOGNITO; accent = '#9775fa'; }
    if (!p) p = dark ? { frame: '#1f2023', toolbar: '#2b2c30', tabActive: '#2b2c30', text: '#e8e9ed', muted: '#a3a7b0', field: '#1c1d20' }
                     : { frame: '#e8ebf0', toolbar: '#ffffff', tabActive: '#ffffff', text: '#1f2328', muted: '#5f6670', field: '#eef1f5' };
    const r = document.documentElement.style;
    const set = (k, v) => r.setProperty(k, v);
    set('--frame', p.frame);
    set('--toolbar', p.toolbar);
    set('--tab-active', p.tabActive);
    set('--text', p.text);
    set('--muted', p.muted);
    set('--field', p.field);
    set('--accent', accent);
    set('--on-accent', onAccent(accent));
    set('--accent-soft', mix(p.toolbar, accent, dark ? 0.28 : 0.14));
    set('--border', mix(p.toolbar, p.text, dark ? 0.16 : 0.13));
    set('--hover', mix(p.frame, p.text, dark ? 0.1 : 0.07));
    set('--hover-strong', mix(p.frame, p.text, dark ? 0.16 : 0.12));
    set('--tb-hover', mix(p.toolbar, p.text, dark ? 0.1 : 0.07));
    set('--popup', dark ? mix(p.toolbar, '#ffffff', 0.04) : '#ffffff');
    set('--page', dark ? mix(p.frame, '#000000', 0.25) : mix(p.frame, '#ffffff', 0.6));
    set('--card', dark ? p.toolbar : '#ffffff');
    set('--danger', dark ? '#ff8787' : '#c92a2a');
    set('--ok', dark ? '#69db7c' : '#2b8a3e');
    set('--warn', dark ? '#ffd43b' : '#b35c00');
    const d = DENSITY[t.density] || DENSITY.comfortable;
    set('--tab-h', d.tab + 'px');
    set('--toolbar-h', d.toolbar + 'px');
    set('--bm-h', d.bm + 'px');
    set('--font', d.font + 'px');
    set('--radius', d.radius + 'px');
    set('--gap', d.gap + 'px');
    const root = document.documentElement;
    root.dataset.dark = dark ? '1' : '0';
    root.dataset.density = t.density || 'comfortable';
    root.dataset.incognito = t.incognito ? '1' : '0';
    root.classList.toggle('no-anim', t.animations === false);
    root.style.colorScheme = dark ? 'dark' : 'light';
    let css = document.getElementById('solus-custom-css');
    if (!css) { css = document.createElement('style'); css.id = 'solus-custom-css'; document.head.appendChild(css); }
    css.textContent = t.customCss || '';
    window.solusTheme = { ...t, palette: p, accent, density: d };
    document.dispatchEvent(new CustomEvent('solus-theme', { detail: window.solusTheme }));
  };
  window.solusThemeUtil = { mix, contrast, onAccent };
})();
