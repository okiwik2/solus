//! Test automation channel (used by the integration/UI test suite and benchmarks).
//!
//! Only active when Solus is started with `--automation-port=N` *and* the environment variable
//! `SOLUS_AUTOMATION_TOKEN` is set; every request must carry that token. Listens on 127.0.0.1 only.
//! Protocol: one JSON object per line in, one JSON object per line out.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::Duration;

use base64::Engine;
use serde_json::{json, Value};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::Win32::UI::Shell::SHCreateMemStream;

use crate::app::{post, App};

pub fn start(port: u16) {
    let Ok(token) = std::env::var("SOLUS_AUTOMATION_TOKEN") else {
        logf!("automation port given but SOLUS_AUTOMATION_TOKEN is not set; automation disabled");
        return;
    };
    if token.len() < 16 {
        logf!("automation token too short; automation disabled");
        return;
    }
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            logf!("automation bind failed: {e}");
            return;
        }
    };
    logf!("automation listening on 127.0.0.1:{port}");
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let token = token.clone();
            std::thread::spawn(move || serve(s, &token));
        }
    });
}

fn serve(stream: TcpStream, token: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(600)));
    let mut w = stream.try_clone().unwrap();
    let r = BufReader::new(stream);
    for line in r.lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
        let resp = if req["token"].as_str() != Some(token) {
            json!({"ok": false, "error": "bad token"})
        } else {
            let (tx, rx) = mpsc::channel::<Value>();
            post(move |app| handle(app, req, tx));
            rx.recv_timeout(Duration::from_secs(120)).unwrap_or(json!({"ok": false, "error": "timeout"}))
        };
        if w.write_all(format!("{resp}\n").as_bytes()).is_err() {
            break;
        }
    }
}

fn target_wv(app: &App, req: &Value) -> Option<ICoreWebView2> {
    let w = match req["win"].as_u64() {
        Some(id) => app.win(id as u32),
        None => app.windows.iter().filter(|w| !w.closing).max_by_key(|w| w.last_focus),
    }?;
    match &req["target"] {
        Value::String(s) if s == "ui" => w.ui.as_ref().map(|v| v.wv.clone()),
        Value::String(s) if s == "active" => w.active_tab().and_then(|t| t.view.as_ref()).map(|v| v.wv.clone()),
        Value::Number(n) => w.tab(n.as_u64()? as u32).and_then(|t| t.view.as_ref()).map(|v| v.wv.clone()),
        _ => w.active_tab().and_then(|t| t.view.as_ref()).map(|v| v.wv.clone()),
    }
}

fn handle(app: &mut App, req: Value, tx: mpsc::Sender<Value>) {
    let cmd = req["cmd"].as_str().unwrap_or("");
    match cmd {
        "state" => {
            let wins: Vec<Value> = app
                .windows
                .iter()
                .map(|w| {
                    json!({
                        "id": w.id, "profile": w.profile, "incognito": w.incognito, "active": w.active,
                        "visible": w.is_visible(), "uiReady": w.ui_ready, "workspace": w.workspace,
                        "split": w.split.as_ref().map(|s| json!([s.a, s.b])),
                        "tabs": w.tabs.iter().map(|t| t.to_json()).collect::<Vec<_>>(),
                        "groups": crate::bridge::groups_json(w),
                    })
                })
                .collect();
            let _ = tx.send(json!({"ok": true, "windows": wins, "pid": std::process::id()}));
        }
        "eval" => {
            let Some(wv) = target_wv(app, &req) else {
                let _ = tx.send(json!({"ok": false, "error": "no such webview"}));
                return;
            };
            let script = req["script"].as_str().unwrap_or("").to_string();
            crate::webview::exec_script(&wv, &script, move |_, r| {
                let v = r.and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or(Value::Null);
                let _ = tx.send(json!({"ok": true, "result": v}));
            });
        }
        "capture" => {
            let Some(wv) = target_wv(app, &req) else {
                let _ = tx.send(json!({"ok": false, "error": "no such webview"}));
                return;
            };
            let path = req["path"].as_str().unwrap_or("capture.png").to_string();
            unsafe {
                let stream = SHCreateMemStream(None).unwrap();
                let s2 = stream.clone();
                let h = CapturePreviewCompletedHandler::create(Box::new(move |hr| {
                    let bytes = crate::tab::read_stream_from_start(&s2);
                    let ok = hr.is_ok() && std::fs::write(&path, &bytes).is_ok();
                    let _ = tx.send(json!({"ok": ok, "bytes": bytes.len()}));
                    Ok(())
                }));
                let _ = wv.CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &stream, &h);
            }
        }
        "bridge" => {
            // invoke a bridge command as if the browser UI sent it
            let wid = req["win"].as_u64().map(|x| x as u32).or_else(|| app.last_active_window(false)).unwrap_or(0);
            let msg = json!({"id": null, "cmd": req["name"], "args": req["args"]});
            crate::bridge::run_for_automation(app, wid, msg, Box::new(move |r| {
                let _ = tx.send(json!({"ok": true, "result": r}));
            }));
        }
        "action" => {
            let wid = req["win"].as_u64().map(|x| x as u32).or_else(|| app.last_active_window(false)).unwrap_or(0);
            crate::bridge::run_action(app, wid, req["name"].as_str().unwrap_or(""), req["data"].clone());
            let _ = tx.send(json!({"ok": true}));
        }
        "cdp" => {
            let Some(wv) = target_wv(app, &req) else {
                let _ = tx.send(json!({"ok": false, "error": "no such webview"}));
                return;
            };
            let m = crate::util::WStr::new(req["method"].as_str().unwrap_or(""));
            let p = crate::util::WStr::new(&req["params"].to_string());
            let h = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |hr, result| {
                let _ = tx.send(json!({"ok": hr.is_ok(), "result": serde_json::from_str::<Value>(&result).unwrap_or(json!(result))}));
                Ok(())
            }));
            unsafe {
                let _ = wv.CallDevToolsProtocolMethod(m.pcwstr(), p.pcwstr(), &h);
            }
        }
        "memory" => {
            let _ = tx.send(json!({"ok": true, "memory": crate::perf::memory_snapshot(app)}));
        }
        "quit" => {
            let _ = tx.send(json!({"ok": true}));
            app.quit();
        }
        _ => {
            let _ = tx.send(json!({"ok": false, "error": format!("unknown {cmd}")}));
        }
    }
}

#[allow(dead_code)]
fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}
