#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(clippy::too_many_arguments, clippy::new_without_default)]

#[macro_use]
pub mod util;
pub mod ai;
pub mod app;
pub mod automation;
pub mod backup;
pub mod bridge;
pub mod caption;
pub mod content;
pub mod crypto;
pub mod downloads;
pub mod extensions;
pub mod hooks;
pub mod import;
pub mod net;
pub mod omnibox;
pub mod passwords;
pub mod perf;
pub mod platform;
pub mod privacy;
pub mod productivity;
pub mod scheme;
pub mod settings;
pub mod store;
pub mod sync;
pub mod tab;
pub mod themes;
pub mod translate;
pub mod update;
pub mod webview;
pub mod window;
pub mod winauth;

use std::cell::RefCell;
use std::path::PathBuf;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const APP_ID: &str = "Solus.Browser";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Args {
    pub user_data_dir: Option<PathBuf>,
    pub urls: Vec<String>,
    pub incognito: bool,
    pub new_window: bool,
    pub automation_port: Option<u16>,
    pub background: bool,
    pub no_restore: bool,
    pub profile: Option<String>,
}

thread_local! {
    pub static ARGS: RefCell<Args> = RefCell::new(Args::default());
}

fn parse_args() -> Args {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(s) = it.next() {
        if let Some(v) = s.strip_prefix("--user-data-dir=") {
            a.user_data_dir = Some(PathBuf::from(v));
        } else if let Some(v) = s.strip_prefix("--automation-port=") {
            a.automation_port = v.parse().ok();
        } else if let Some(v) = s.strip_prefix("--profile=") {
            a.profile = Some(v.to_string());
        } else if s == "--incognito" || s == "-inprivate" {
            a.incognito = true;
        } else if s == "--new-window" {
            a.new_window = true;
        } else if s == "--background" {
            a.background = true;
        } else if s == "--no-restore" {
            a.no_restore = true;
        } else if s == "--" {
            a.urls.extend(it.by_ref());
        } else if !s.starts_with("--") {
            a.urls.push(s);
        }
    }
    a
}

/// If another Solus is running for the same data folder, hand our arguments to it and return true.
fn forward_to_running_instance(args: &Args) -> bool {
    let key = instance_key();
    let name = util::WStr::new(&format!("Local\\SolusBrowser-{key}"));
    unsafe {
        let _m = CreateMutexW(None, true, name.pcwstr());
        if GetLastError() != ERROR_ALREADY_EXISTS {
            // keep the mutex handle alive for the life of the process
            std::mem::forget(_m);
            return false;
        }
        let title = util::WStr::new(&format!("SolusMsg-{key}"));
        for _ in 0..50 {
            if let Ok(h) = FindWindowW(w!("SolusMsgWindow"), title.pcwstr()) {
                if !h.0.is_null() {
                    let payload = serde_json::to_vec(args).unwrap_or_default();
                    let cds = COPYDATASTRUCT {
                        dwData: 0x5301,
                        cbData: payload.len() as u32,
                        lpData: payload.as_ptr() as *mut _,
                    };
                    // let the running instance bring its window to the foreground
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(h, Some(&mut pid));
                    let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
                    let mut res = 0usize;
                    let r = SendMessageTimeoutW(
                        h,
                        WM_COPYDATA,
                        WPARAM(0),
                        LPARAM(&cds as *const _ as isize),
                        SMTO_ABORTIFHUNG | SMTO_BLOCK,
                        8000,
                        Some(&mut res),
                    );
                    if r.0 == 0 {
                        let msg = util::WStr::new("Solus is still closing from last time. Please try again in a few seconds.");
                        MessageBoxW(None, msg.pcwstr(), w!("Solus"), MB_OK | MB_ICONINFORMATION);
                    }
                    return true;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        true
    }
}

pub fn instance_key() -> String {
    use sha2::Digest;
    let root = util::user_data_root();
    let h = sha2::Sha256::digest(root.to_string_lossy().to_lowercase().as_bytes());
    h[..6].iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    let args = parse_args();
    ARGS.with(|a| *a.borrow_mut() = args.clone());
    util::init_log();
    if forward_to_running_instance(&args) {
        return;
    }
    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        util::log(&format!("PANIC: {info}\n{bt}"));
    }));
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let id = util::WStr::new(APP_ID);
        let _ = SetCurrentProcessExplicitAppUserModelID(id.pcwstr());
    }
    logf!("Solus {} starting, data={}", VERSION, util::user_data_root().display());
    app::run(args);
}
