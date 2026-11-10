# Testing Solus

| Suite | Command | What it covers |
|---|---|---|
| Unit | `cargo test --bin solus` | Encryption (AES-GCM, HMAC vectors, DPAPI, Argon2), encrypted history search, bookmark tree operations, notes/tasks/reminders, Chrome bookmark + history import (Chrome schemas), password CSV import (Chrome/Edge/Firefox/Bitwarden) and export, sync vault + two-device merge, ad-blocking against the real EasyList/EasyPrivacy, extension package unpacking (zip-slip refused), update signature checks, omnibox input handling, HTTPS-First rules, internal-page path safety, translation language detection. |
| End-to-end | `node tests/integration.mjs [exe]` | Starts a real build with a throwaway profile and drives it through the automation channel: first-run wizard, UI rendering, navigation, search fallback, HTTPS-First, web pages blocked from internal pages, encrypted history on disk, bookmarks, tabs/reopen/move, groups, auto-grouping, workspaces, split screen, ad + cosmetic blocking, per-site exceptions, downloads, password save + fill, permission prompts, translation offer, find in page, freeze/unload/restore, renderer crash recovery, Incognito not recording history, every internal page loading, AI key handling, settings persistence, encrypted sync files, backups, session restore after quit and after a forced kill. |
| End-to-end (2) | `node tests/integration2.mjs [exe]` | The browser UI is actually visible and the page area has the right size (regression test for 1.0.0), third-party cookie blocking (with an "allowed" control run), profile isolation, reminders firing, installing and removing a real Chrome Web Store extension, task-manager process→tab mapping. |
| Installer | `node tests/installer.mjs dist\SolusSetup-x.y.z.exe` | Silent per-user install, Start-menu shortcut, default-browser registration, no auto-start, installed copy browses with bundled filter lists, upgrade over the top keeps data, uninstall removes files/registration and keeps user data. |
| Updates | `node tests/update.mjs [exe] [installer]` | A correctly signed update is accepted; a tampered installer, a forged signature and a foreign download host are refused. |
| Memory | `node bench/memory.mjs` | Solus vs Chrome, fresh profiles, same sites; private working set of each browser's whole process tree at fixed times. Results in `bench/results/`. |
| Live update feed | `node tests/_livefeed.mjs [exe]` | The build parses the published GitHub feed (network). |
| Real update | `node tests/_realupdate.mjs [installed exe]` | Runs the installed copy (throwaway profile) against the live feed, lets it download and verify the newest release, quits, and checks the installed version was upgraded. Used for 1.0.0 → 1.0.1. |
| Visual tour | `node tests/_tour.mjs [--dark]` | Screenshots of the real window (page, menu over page, new tab, sidebar, settings, split view) into `%SHOT_DIR%`. Needs an unlocked screen. |
| Startup & CPU | `node bench/cpu-startup.mjs` | Time to a visible window (median of 5) and background CPU with 10 sites open and no input. |

The automation channel used by the tests is compiled into every build but is inert unless Solus is started with `--automation-port=N` **and** the `SOLUS_AUTOMATION_TOKEN` environment variable; it listens on 127.0.0.1 only and every request must carry the token.

Notes
- EasyList turns off generic cosmetic filtering on `localhost`/`127.0.0.1`, so the ad test serves its page as `ads.solus.test` (mapped to the local test server with `--host-resolver-rules`).
- When the Windows session is locked, Chromium treats every page as hidden and stops producing frames; the end-to-end tests check DOM state rather than pixels.
- Lesson from 1.0.0: DOM checks alone missed that the UI layer was never painted. Before a release, run `tests/_tour.mjs` on an unlocked screen and look at the screenshots; `tests/_vis.mjs` and `tools/wintree.ps1` help diagnose visibility and window-layering problems.
