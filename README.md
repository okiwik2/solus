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

## Measured memory and CPU (bench/results)
Intel Core Ultra 5 338H, 16 GB, on mains power. Chrome 157 (fresh profile, default settings) vs Solus 1.0.1 (fresh profile) with each performance profile pinned. Memory is the private working set of each browser's whole process tree; the same 10 sites (Wikipedia, BBC, GitHub, YouTube, Reddit, Stack Overflow, Guardian, NYT, Amazon, Stuff) are opened in each.

| | Chrome | Solus — Balanced (default on mains) | Solus — Efficiency (default on battery) |
|---|---|---|---|
| 1 tab | 305–348 MB | 167 MB | 166 MB |
| 10 sites after 45 s | 1,400 MB | 436 MB | 835 MB |
| 10 sites after 3 min | 1,346 MB | 710 MB | 430 MB |
| 10 sites after 6 min | 1,151–1,321 MB | 391 MB | 311 MB |
| Background CPU, 10 sites idle for 3 min | 34.5–34.8 CPU-s (1.6% of the machine) | 21.1 CPU-s (0.98%) | 11.9 CPU-s (0.55%) |
| Startup to visible window (median of 5) | 250–259 ms | 89 ms | 93 ms |

Chrome was measured alongside each Solus run, hence the ranges. Single runs on live websites are noisy (ads and video differ from minute to minute; the Balanced 45 s reading caught pages mid-load), so treat the differences as indicative rather than exact. Run `node bench/memory.mjs --profile=balanced|efficiency` and `node bench/cpu-startup.mjs --profile=...` to reproduce.

Where the saving comes from: Solus asks the engine to shrink background tabs after 30 s (Balanced) and pauses them after 5 min; Efficiency pauses after 1 min and unloads tabs when memory use passes 75%. Chrome's Memory Saver only discards tabs much later. "Visible window" is when the top-level window appears; the toolbar and page fill in a moment later, so it measures perceived launch, not page load.

> Correction: the figures published with 1.0.0 (e.g. "298 MB after 3 min") were labelled Balanced but were taken on battery, where Solus switches to Efficiency automatically. The benchmarks now pin the profile and record the power source.

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
