# Solus

A low-memory Chromium browser for Windows 11, written in Rust on the Microsoft Edge WebView2 runtime.

## Architecture

| Layer | What | Where |
|---|---|---|
| Engine | Chromium via the Evergreen **WebView2** runtime (patched automatically by Windows). One shared browser process for all windows and profiles; one webview per loaded tab. | `src/webview.rs` |
| Host | Native Win32 app (Rust, `windows` + `webview2-com`). Single UI thread; COM callbacks only queue work (`app::defer`), so state is never re-entered. | `src/app.rs`, `src/window.rs`, `src/tab.rs` |
| Browser UI | Plain HTML/CSS/JS (no framework) in one small webview per window, layered above the tab webviews. Its window region is "everything except the page + open popups", so menus and the address-bar dropdown draw over live pages without screenshots. Caption buttons are native so Windows 11 Snap Layouts work. | `ui/chrome/` |
| Internal pages | `solus://` custom scheme served from `ui/pages/`. Web pages cannot navigate to, frame or fetch them; their bridge only accepts messages whose source is a `solus://` document. | `src/scheme.rs`, `src/bridge.rs` |
| Content scripts | Run in an isolated JavaScript world ("solus") and report through a DevTools binding that exists only in that world (password capture, language detection, unsaved-form detection). | `src/content.rs` |
| Storage | Per-profile SQLite (WAL). History URLs/titles, notes and passwords are AES-256-GCM encrypted with a DPAPI-protected profile key. | `src/store.rs`, `src/crypto.rs` |

### Feature modules
`perf.rs` (tab freeze/unload, memory pressure, battery, prerender, task-manager data) · `privacy.rs` (adblock-rust + EasyList/EasyPrivacy, cosmetic filtering, permissions, data clearing, dashboard) · `passwords.rs` · `import.rs` (Chrome) · `ai.rs` (Claude Messages API with streaming + tools; Groq option) · `sync.rs` (E2E-encrypted folder sync + cloud backups) · `backup.rs` · `extensions.rs` (Chrome Web Store) · `translate.rs` · `update.rs` (Ed25519-signed updates) · `productivity.rs` (workspaces, groups, notes, tasks, reminders) · `downloads.rs` · `automation.rs` (test channel, disabled unless started with a port **and** a token).

## Build
Requirements: Rust (stable), Visual Studio Build Tools (MSVC), Inno Setup 6, Node.js (tests only).

```
cargo build --release                      # target\release\solus.exe
powershell -File tools\build-release.ps1    # dist\SolusSetup-<version>.exe (+ signed latest.json)
```

## Tests
```
cargo test --bin solus                      # unit tests (crypto, storage, import, sync merge, blocking, ...)
node tests\integration.mjs                  # end-to-end tests against a real build (throwaway profile)
node tests\integration2.mjs                 # third-party cookies, profiles, reminders, Web Store extension, task manager
node tests\installer.mjs                    # installs dist\SolusSetup-*.exe per-user, upgrades, uninstalls
node tests\update.mjs                       # signed-update acceptance and tamper/forgery refusal
node bench\memory.mjs                       # Solus vs Chrome memory benchmark
node bench\cpu-startup.mjs                  # startup time and background CPU vs Chrome
```

## Measured memory (bench/results)
Intel Core Ultra 5 338H, 16 GB, Chrome 157 (fresh profile, defaults) vs Solus 1.0 (Balanced). Private working set of each browser's whole process tree, same 10 sites (Wikipedia, BBC, GitHub, YouTube, Reddit, Stack Overflow, Guardian, NYT, Amazon, Stuff).

| | Chrome | Solus |
|---|---|---|
| 1 tab | 309 MB | 152 MB (−51%) |
| 10 sites after 45 s | 1,424 MB | 1,013 MB (−29%) |
| 10 sites after 3 min | 1,406 MB | 298 MB (−79%) |
| 10 sites after 6 min | 1,370 MB | 342 MB (−75%) |

### Startup and background CPU (`bench/cpu-startup.mjs`)
| | Chrome | Solus |
|---|---|---|
| Startup to visible main window, median of 5 | 269 ms | 89 ms |
| Background CPU, 10 sites idle for 3 min | 31.8 CPU-s (1.47% of the machine) | 4.3 CPU-s (0.2%) |

"Visible window" is when the top-level window appears; Solus shows its window frame immediately and the toolbar/page fill in a moment later, so this measures perceived launch, not page load. Background CPU is lower mainly because idle background tabs are paused.

Most of the saving comes from Solus asking the engine to shrink background tabs and pausing them; Chrome's own Memory Saver only discards tabs after much longer. Numbers vary with sites and ads; run `node bench/memory.mjs` to reproduce.

## Known limitations (and what Solus does instead)
- **Engine is WebView2, not a Chromium fork.** Chromium security updates arrive automatically through Windows. Malicious-site protection is **Microsoft Defender SmartScreen**; Google Safe Browsing isn't available to Chromium-based apps outside Google's own browser.
- **Chrome's password manager can't be shared.** Since Chrome 127 its passwords (and cookies) are protected by App-Bound Encryption that only Chrome can unlock. Solus imports passwords through Chrome's own CSV export and keeps them in its own encrypted vault. Bookmarks, history and search shortcuts are imported from copies of Chrome's files; Chrome is never modified.
- **Chrome Web Store extensions:** WebView2 supports a subset of the extension APIs (no toolbar action icons; popups/options open as tabs). Many popular extensions work; some won't.
- **Third-party cookie blocking** uses the engine's own third-party-cookie phase-out switch (a preference-only approach turned out to be ignored by WebView2). It is applied when the engine starts, so changing the setting takes effect after restarting Solus, and per-site exceptions don't re-enable third-party cookies for a single site.
- **Session restore** reopens each tab's page but not its back/forward history (WebView2 has no API to serialise it).
- **Translation** uses Google Translate's public web proxy, so it works for public pages but not for pages that need you to be signed in.
- **Windows Hello** (PIN/fingerprint/face) must be set up to reveal or export saved passwords.
- **Content scripts** keep the DevTools Runtime domain enabled per tab (needed for the isolated-world binding); pages that log heavily to the console pay a small extra cost.
- **Unsigned installer:** without a code-signing certificate (paid), Windows SmartScreen shows "Windows protected your PC" on first run — choose *More info → Run anyway*.
- **Automatic updates** need a published release feed (GitHub Releases); until one is published, the update check reports that no feed exists.
- **Web messaging object:** `window.chrome.webview` remains visible to web pages (WebView2 behaviour); messages from web pages are ignored.
