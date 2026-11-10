//! Content scripts for web pages, run in an isolated JavaScript world ("solus") like Chrome
//! extension content scripts. Page scripts can't see or call them. They report back through a
//! DevTools-protocol binding that exists only inside that isolated world.

use serde_json::{json, Value};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::core::PWSTR;

use crate::app::{defer, App};
use crate::util::WStr;

const BINDING: &str = "__solusSend";
const WORLD: &str = "solus";

const SCRIPT: &str = r#"
(() => {
  if (window.__solusCS) return; window.__solusCS = true;
  // the binding is installed into this world by the engine; look it up when needed
  const send = (m) => { try { globalThis.__solusSend(JSON.stringify(m)); } catch (e) {} };
  const visible = (el) => !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length);
  const pwFields = (root) => [...(root || document).querySelectorAll('input[type=password]')].filter((e) => e.value && visible(e));
  let lastSent = '';
  function capture(root) {
    const pws = pwFields(root);
    if (!pws.length) return;
    const pw = pws[pws.length - 1].value;
    const scope = root || pws[0].form || document;
    const named = scope.querySelector('input[autocomplete~=username],input[autocomplete~=email]');
    const texts = [...scope.querySelectorAll('input')].filter((i) => ['text', 'email', 'tel', ''].includes(i.type) && i.value && visible(i));
    const user = (named && named.value) || (texts.length ? texts[texts.length - 1].value : '');
    const key = user + '\u0000' + pw;
    if (key === lastSent) return;
    lastSent = key;
    send({ t: 'pw', origin: location.origin, user: user.slice(0, 200), pw: pw.slice(0, 500), top: window === top });
  }
  document.addEventListener('submit', (e) => capture(e.target), true);
  document.addEventListener('click', (e) => {
    const b = e.target.closest && e.target.closest('button,input[type=submit],input[type=button],[role=button]');
    if (!b) return;
    const f = b.closest('form');
    if (pwFields(f || document).length) setTimeout(() => capture(f), 0);
  }, true);
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && e.target && e.target.type === 'password') capture(e.target.form);
  }, true);
  let dirty = false;
  document.addEventListener('input', (e) => {
    if (!dirty && e.isTrusted && e.target && e.target.type !== 'password' && e.target.type !== 'search') { dirty = true; send({ t: 'dirty' }); }
  }, true);
  if (window === top) {
    const report = () => send({ t: 'lang', lang: (document.documentElement.lang || '').trim(), sample: ((document.body && document.body.innerText) || '').slice(0, 800) });
    if (document.readyState === 'loading') addEventListener('DOMContentLoaded', report, { once: true }); else report();
  }
})();
"#;

fn cdp(wv: &ICoreWebView2, method: &str, params: Value) {
    let m = WStr::new(method);
    let p = WStr::new(&params.to_string());
    let name = method.to_string();
    let h = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |hr, result| {
        if hr.is_err() {
            logf!("content script setup: {name} failed: {hr:?} {result}");
        }
        Ok(())
    }));
    unsafe {
        let _ = wv.CallDevToolsProtocolMethod(m.pcwstr(), p.pcwstr(), &h);
    }
}

pub fn install(wv: &ICoreWebView2, wid: u32, tid: u32) {
    // document scripts only take effect once the Page domain is enabled, and a binding scoped to a
    // named isolated world is only exposed while the Runtime domain is enabled
    cdp(wv, "Page.enable", json!({}));
    cdp(wv, "Runtime.enable", json!({}));
    cdp(wv, "Runtime.addBinding", json!({"name": BINDING, "executionContextName": WORLD}));
    cdp(wv, "Page.addScriptToEvaluateOnNewDocument", json!({"source": SCRIPT, "worldName": WORLD}));
    unsafe {
        let ev = WStr::new("Runtime.bindingCalled");
        let Ok(recv) = wv.GetDevToolsProtocolEventReceiver(ev.pcwstr()) else { return };
        let h = DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut p = PWSTR::null();
            if args.ParameterObjectAsJson(&mut p).is_err() {
                return Ok(());
            }
            let text = take_pwstr(p);
            let Ok(v) = serde_json::from_str::<Value>(&text) else { return Ok(()) };
            if v["name"] != BINDING {
                return Ok(());
            }
            let Ok(msg) = serde_json::from_str::<Value>(v["payload"].as_str().unwrap_or("")) else { return Ok(()) };
            defer(move |app| on_message(app, wid, tid, msg));
            Ok(())
        }));
        let mut tok = Default::default();
        let _ = recv.add_DevToolsProtocolEventReceived(&h, &mut tok);
    }
}

fn on_message(app: &mut App, wid: u32, tid: u32, m: Value) {
    match m["t"].as_str().unwrap_or("") {
        "pw" => crate::passwords::captured(
            app,
            wid,
            tid,
            m["origin"].as_str().unwrap_or(""),
            m["user"].as_str().unwrap_or(""),
            m["pw"].as_str().unwrap_or(""),
        ),
        "lang" => crate::translate::detected(app, wid, tid, m["lang"].as_str().unwrap_or(""), m["sample"].as_str().unwrap_or("")),
        "dirty" => {
            if let Some(t) = app.win_mut(wid).and_then(|w| w.tab_mut(tid)) {
                t.has_input = true;
            }
        }
        _ => {}
    }
}
