// AI assistant: docked side panel and floating popup (both in the browser UI layer).
'use strict';
(() => {
  const { $, $$, h, esc, call, toast } = window.UI;
  const panel = $('#ai-panel');
  let status = null;
  let conv = null;
  let busy = false;
  let mode = null; // 'panel' | 'float'
  let width = 380;
  let host = null; // element currently hosting the chat
  let usage = null;
  const transcript = []; // {role, html}

  // ---------------------------------------------------------------- markdown (safe subset)
  function md(src) {
    const blocks = [];
    let text = String(src || '').replace(/```(\w*)\n?([\s\S]*?)```/g, (_, lang, code) => { blocks.push(`<pre><code>${esc(code.replace(/\n$/, ''))}</code></pre>`); return `\u0000${blocks.length - 1}\u0000`; });
    text = esc(text);
    const inline = (s) => s
      .replace(/`([^`]+)`/g, '<code>$1</code>')
      .replace(/\*\*([^*]+)\*\*/g, '<b>$1</b>')
      .replace(/(^|[^*])\*([^*\n]+)\*/g, '$1<i>$2</i>')
      .replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, (_, t, u) => `<a href="#" data-href="${u}">${t}</a>`);
    const lines = text.split('\n');
    let html = '', list = null;
    const close = () => { if (list) { html += `</${list}>`; list = null; } };
    let para = [];
    const flush = () => { if (para.length) { html += `<p>${inline(para.join('<br>'))}</p>`; para = []; } };
    for (const line of lines) {
      let m;
      if (/^\u0000\d+\u0000$/.test(line.trim())) { flush(); close(); html += line.trim(); continue; }
      if ((m = line.match(/^(#{1,4})\s+(.*)/))) { flush(); close(); html += `<p><b>${inline(m[2])}</b></p>`; continue; }
      if ((m = line.match(/^\s*[-*•]\s+(.*)/))) { flush(); if (list !== 'ul') { close(); html += '<ul>'; list = 'ul'; } html += `<li>${inline(m[1])}</li>`; continue; }
      if ((m = line.match(/^\s*\d+[.)]\s+(.*)/))) { flush(); if (list !== 'ol') { close(); html += '<ol>'; list = 'ol'; } html += `<li>${inline(m[1])}</li>`; continue; }
      if (!line.trim()) { flush(); close(); continue; }
      close(); para.push(line);
    }
    flush(); close();
    return html.replace(/\u0000(\d+)\u0000/g, (_, i) => blocks[+i]);
  }

  // ---------------------------------------------------------------- shell
  function chatHtml(floating) {
    return `<div class="panel-head" data-drag>${icon('sparkle', 16)}<span class="title" style="margin-left:6px">Assistant</span>
      <span class="ai-model" style="color:var(--muted);font-size:11.5px;margin-right:6px"></span>
      <button class="x-btn" data-new title="New chat">${icon('plus', 15)}</button>
      <button class="x-btn" data-swap title="${floating ? 'Dock to sidebar' : 'Pop out as floating window'}">${icon(floating ? 'sidebarRight' : 'external', 15)}</button>
      <button class="x-btn" data-x title="Close">${icon('close', 15)}</button></div>
      <div class="ai-body" style="display:flex;flex-direction:column;flex:1;min-height:0"></div>`;
  }

  function setupHtml() {
    return `<div style="padding:16px;overflow:auto">
      <h3 style="margin:0 0 6px;font-size:15px">Set up the assistant</h3>
      <p style="color:var(--muted);margin:0 0 12px;line-height:1.5">The assistant can summarise and explain pages, answer questions about them, compare tabs, and organise your tabs, bookmarks and workspaces. It uses your own API key, stored encrypted on this PC.</p>
      <label style="font-size:12px;color:var(--muted)">Service</label>
      <select class="field" data-prov style="width:100%;height:32px;margin:4px 0 10px"><option value="anthropic">Claude (Anthropic) — recommended</option><option value="groq">Groq — free tier, less capable</option></select>
      <label style="font-size:12px;color:var(--muted)">API key</label>
      <input class="field" data-key type="password" style="width:100%;height:32px;margin:4px 0 6px" placeholder="sk-ant-…" autocomplete="off">
      <div style="font-size:12px;color:var(--muted);margin-bottom:12px" data-help></div>
      <button class="btn primary" data-save style="width:100%;height:34px">Save key</button>
      <p style="font-size:11.5px;color:var(--muted);margin-top:12px;line-height:1.5">Claude API usage is billed by Anthropic to your account (separate from a Claude.ai subscription). What the assistant can see and do is controlled in Settings &gt; AI assistant.</p></div>`;
  }

  async function refreshStatus() {
    try { status = await call('ai.status'); } catch { status = null; }
    return status;
  }

  async function mount(where) {
    mode = where;
    await refreshStatus();
    if (where === 'panel') {
      $$('.ai-float').forEach((x) => x.remove());
      panel.hidden = false;
      panel.innerHTML = chatHtml(false);
      host = panel;
      document.documentElement.style.setProperty('--ai-w', width + 'px');
      $('#ai-btn').classList.add('on');
    } else {
      panel.hidden = true; panel.innerHTML = '';
      document.documentElement.style.setProperty('--ai-w', '0px');
      $$('.ai-float').forEach((x) => x.remove());
      const f = h(`<div class="popup ai-float panel" style="width:400px;height:520px;right:20px;top:${($('#page-host').getBoundingClientRect().top + 16)}px;display:flex;flex-direction:column;border-radius:12px;overflow:hidden">${chatHtml(true)}</div>`);
      $('#layer').appendChild(f);
      host = f;
      dragFloat(f);
      $('#ai-btn').classList.add('on');
    }
    wire();
    render();
    requestAnimationFrame(UI.reportLayout);
  }

  function unmount() {
    panel.hidden = true; panel.innerHTML = '';
    $$('.ai-float').forEach((x) => x.remove());
    document.documentElement.style.setProperty('--ai-w', '0px');
    mode = null; host = null;
    $('#ai-btn').classList.remove('on');
    requestAnimationFrame(UI.reportLayout);
  }

  function dragFloat(f) {
    const head = $('[data-drag]', f);
    head.addEventListener('mousedown', (e) => {
      if (e.target.closest('button')) return;
      const r = f.getBoundingClientRect();
      const dx = e.clientX - r.left, dy = e.clientY - r.top;
      const move = (ev) => {
        f.style.right = 'auto';
        f.style.left = Math.max(0, Math.min(innerWidth - r.width, ev.clientX - dx)) + 'px';
        f.style.top = Math.max(0, Math.min(innerHeight - 60, ev.clientY - dy)) + 'px';
        UI.reportLayout();
      };
      const up = () => { document.removeEventListener('mousemove', move); document.removeEventListener('mouseup', up); };
      document.addEventListener('mousemove', move); document.addEventListener('mouseup', up);
    });
  }

  function wire() {
    $('[data-x]', host).onclick = () => unmount();
    $('[data-new]', host).onclick = () => newChat();
    $('[data-swap]', host).onclick = () => mount(mode === 'panel' ? 'float' : 'panel');
  }

  function render() {
    if (!host) return;
    const body = $('.ai-body', host);
    const model = status && status.models && (status.models[status.provider] || []).find((m) => m.id === status.model);
    $('.ai-model', host).textContent = model ? model.name.replace(/\s*\(.*\)/, '') : '';
    if (!status) { body.innerHTML = '<div class="empty">The assistant is unavailable.</div>'; return; }
    if (!status.enabled) { body.innerHTML = `<div class="empty">The assistant is turned off.<br><br><button class="btn" data-settings>Open AI settings</button></div>`; $('[data-settings]', body).onclick = () => call('openInternal', { page: 'settings/#ai' }); return; }
    if (!status.allowedHere) { body.innerHTML = `<div class="empty">${icon('incognito', 28)}<br>The assistant is off in Incognito windows so your private browsing stays private. You can allow it in AI settings.</div>`; return; }
    if (!status.keySet) { body.innerHTML = setupHtml(); wireSetup(body); return; }
    body.innerHTML = `<div class="ai-msgs"></div><div data-extra></div>
      <div class="ai-quick">${[['sum', 'Summarise page'], ['explain', 'Explain simply'], ['points', 'Key points'], ['compare', 'Compare tabs'], ['organise', 'Organise my tabs']].map(([k, l]) => `<button data-q="${k}">${l}</button>`).join('')}</div>
      <div class="ai-ctx"><label title="Send the current page's text with your message"><input type="checkbox" data-page checked> Use this page</label><span style="flex:1"></span><span data-usage></span></div>
      <div class="ai-input"><textarea rows="1" placeholder="Ask about this page or your tabs…" data-in></textarea><button class="btn primary" data-send style="height:38px">${icon('send', 16)}</button></div>`;
    const msgs = $('.ai-msgs', body);
    msgs.innerHTML = transcript.map((m) => `<div class="ai-msg ${m.role}">${m.html}</div>`).join('') || `<div class="empty" style="padding:20px 8px">${icon('sparkle', 26)}<br>Ask anything about the page you're on, or try a quick action below.</div>`;
    msgs.scrollTop = msgs.scrollHeight;
    const input = $('[data-in]', body);
    input.addEventListener('input', () => { input.style.height = 'auto'; input.style.height = Math.min(160, input.scrollHeight) + 'px'; });
    input.addEventListener('keydown', (e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(input.value); } });
    $('[data-send]', body).onclick = () => (busy ? call('ai.stop', { conv }) : send(input.value));
    $$('[data-q]', body).forEach((b) => (b.onclick = () => quick(b.dataset.q)));
    msgs.addEventListener('click', (e) => { const a = e.target.closest('[data-href]'); if (a) { e.preventDefault(); call('newTab', { url: a.dataset.href }); } });
    setBusy(busy);
    showUsage();
    setTimeout(() => input.focus(), 30);
  }

  function wireSetup(body) {
    const prov = $('[data-prov]', body), key = $('[data-key]', body), help = $('[data-help]', body);
    const upd = () => {
      help.innerHTML = prov.value === 'groq'
        ? 'Get a free key at <a href="#" data-href="https://console.groq.com/keys">console.groq.com/keys</a>.'
        : 'Create a key at <a href="#" data-href="https://console.anthropic.com/settings/keys">console.anthropic.com</a> (Settings → API keys).';
      key.placeholder = prov.value === 'groq' ? 'gsk_…' : 'sk-ant-…';
    };
    prov.value = status.provider || 'anthropic'; upd();
    prov.onchange = upd;
    body.addEventListener('click', (e) => { const a = e.target.closest('[data-href]'); if (a) { e.preventDefault(); call('newTab', { url: a.dataset.href }); } });
    $('[data-save]', body).onclick = async () => {
      try {
        await call('setSetting', { key: 'ai.provider', value: prov.value });
        await call('setSetting', { key: 'ai.model', value: '' });
        await call('ai.setKey', { provider: prov.value, key: key.value });
        $('[data-save]', body).textContent = 'Checking…';
        await call('ai.testKey', { provider: prov.value });
        toast('Key saved and verified.');
        conv = null; await refreshStatus(); render();
      } catch (e) { toast(e.message); $('[data-save]', body).textContent = 'Save key'; }
    };
  }

  function setBusy(b) {
    busy = b;
    if (!host) return;
    const btn = $('[data-send]', host);
    if (btn) { btn.innerHTML = icon(b ? 'stop' : 'send', 16); btn.title = b ? 'Stop' : 'Send'; }
    $$('[data-q]', host).forEach((x) => (x.disabled = b));
  }

  function showUsage() {
    const u = host && $('[data-usage]', host);
    if (!u || !usage) return;
    const k = (n) => (n >= 1000 ? (n / 1000).toFixed(1) + 'k' : n);
    u.textContent = `${k(usage.input + usage.cacheRead)} in · ${k(usage.output)} out · ≈ US$${usage.cost.toFixed(usage.cost < 0.1 ? 3 : 2)}`;
    u.title = 'Tokens used in this chat and the estimated cost at the model\'s list price';
  }

  function append(role, html) {
    transcript.push({ role, html });
    const msgs = host && $('.ai-msgs', host);
    if (!msgs) return null;
    if (transcript.length === 1) msgs.innerHTML = '';
    const el = h(`<div class="ai-msg ${role}">${html}</div>`);
    msgs.appendChild(el);
    msgs.scrollTop = msgs.scrollHeight;
    return el;
  }

  let liveText = '', liveEl = null, toolEl = null;
  function liveMsg() {
    if (!liveEl || !liveEl.isConnected) { liveText = ''; liveEl = append('assistant', '<span class="spinner"></span>'); transcript[transcript.length - 1].live = true; }
    return liveEl;
  }

  async function newChat() {
    transcript.length = 0; usage = null; conv = null;
    try { conv = await call('ai.new'); } catch (e) { toast(e.message); }
    render();
  }

  async function send(text) {
    text = (text || '').trim();
    if (!text || busy) return;
    if (!conv) { try { conv = await call('ai.new'); } catch (e) { toast(e.message); return; } }
    append('user', esc(text).replace(/\n/g, '<br>'));
    const input = host && $('[data-in]', host);
    if (input) { input.value = ''; input.style.height = 'auto'; }
    liveEl = null; liveMsg();
    setBusy(true);
    try {
      await call('ai.send', { conv, text, includePage: host ? $('[data-page]', host)?.checked !== false : true });
    } catch (e) {
      setBusy(false);
      if (liveEl) { liveEl.remove(); transcript.pop(); liveEl = null; }
      append('error', esc(e.message));
    }
  }

  function quick(k) {
    const p = {
      sum: 'Summarise this page in a few bullet points, then one sentence on why it matters.',
      explain: 'Explain what this page is about in simple, plain language, as if to someone new to the topic.',
      points: 'List the key facts, figures and takeaways from this page.',
      compare: 'Compare the pages in my open tabs that cover the same topic. Read the relevant tabs first, then give a short comparison (a table if it helps) and a recommendation.',
      organise: 'Look at my open tabs and organise them into sensible tab groups. Don\'t close anything.',
    }[k];
    if (k === 'compare' || k === 'organise') { const c = host && $('[data-page]', host); if (c) c.checked = false; }
    send(p);
  }

  // ---------------------------------------------------------------- host events
  solus.on('aiDelta', (d) => {
    if (d.conv !== conv) return;
    if (toolEl) { toolEl = null; liveEl = null; }
    const el = liveMsg();
    liveText += d.text;
    el.innerHTML = md(liveText);
    transcript[transcript.length - 1].html = el.innerHTML;
    const msgs = host && $('.ai-msgs', host);
    if (msgs) msgs.scrollTop = msgs.scrollHeight;
  });
  solus.on('aiThinking', (d) => {
    if (d.conv !== conv) return;
    const el = liveMsg();
    if (!liveText) el.innerHTML = '<span class="spinner"></span> <span style="color:var(--muted)">Thinking…</span>';
  });
  solus.on('aiTool', (d) => {
    if (d.conv !== conv) return;
    if (d.state === 'done') return;
    if (liveEl && !liveText) { liveEl.remove(); transcript.pop(); }
    liveEl = null; liveText = '';
    if (d.state === 'start' || !toolEl) toolEl = append('tool', `${icon('tools', 13)} ${esc(d.label || d.name)}…`);
    else toolEl.innerHTML = `${icon('tools', 13)} ${esc(d.label || d.name)}…`;
  });
  solus.on('aiDone', (d) => {
    if (d.conv !== conv) return;
    if (liveEl && !liveText) { liveEl.remove(); transcript.pop(); }
    if (d.stop === 'max_tokens') append('error', 'The reply hit its length limit.');
    liveEl = null; toolEl = null;
  });
  solus.on('aiState', (d) => {
    if (d.conv !== conv) return;
    setBusy(d.busy);
    if (!d.busy && liveEl && !liveText) { liveEl.remove(); transcript.pop(); liveEl = null; }
    if (d.stopped) append('tool', 'Stopped.');
  });
  solus.on('aiError', (d) => {
    if (d.conv !== conv) return;
    if (liveEl && !liveText) { liveEl.remove(); transcript.pop(); }
    liveEl = null;
    append('error', esc(d.error));
  });
  solus.on('aiUsage', (d) => { if (d.conv === conv) { usage = d; showUsage(); } });
  solus.on('aiConfirm', (d) => {
    if (d.conv !== conv || !host) return;
    const extra = $('[data-extra]', host);
    extra.innerHTML = `<div class="ai-confirm"><b>The assistant wants to:</b><div style="margin-top:4px">${esc(d.summary)}</div><div class="actions"><button class="btn" data-no>Don't allow</button><button class="btn primary" data-yes>Allow</button></div></div>`;
    $('[data-yes]', extra).onclick = () => { extra.innerHTML = ''; call('ai.confirm', { conv, approve: true }); };
    $('[data-no]', extra).onclick = () => { extra.innerHTML = ''; call('ai.confirm', { conv, approve: false }); };
  });
  solus.on('aiPermission', (d) => {
    if (!host) return;
    const extra = $('[data-extra]', host);
    extra.innerHTML = `<div class="ai-confirm">The assistant asked to <b>${esc(d.label)}</b>, which you haven't allowed.<div class="actions"><button class="btn" data-no>Not now</button><button class="btn primary" data-yes>Allow</button></div></div>`;
    $('[data-yes]', extra).onclick = () => { extra.innerHTML = ''; call('setSetting', { key: 'ai.permissions.' + d.perm, value: true }).then(() => toast('Allowed. Ask again and the assistant can use it.')); };
    $('[data-no]', extra).onclick = () => { extra.innerHTML = ''; };
  });
  solus.on('settings', () => { if (mode) refreshStatus().then(() => { if (!busy) render(); }); });

  window.AI = {
    init() { width = S.settings.ai_panel_width || 380; },
    toggle() { mode === 'panel' ? unmount() : mount('panel'); },
    popup() { mode === 'float' ? unmount() : mount('float'); },
    ask(text) { (mode ? Promise.resolve() : mount('panel')).then(() => send(text)); },
  };
})();
