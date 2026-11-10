# Solus — user guide

## Getting started
- **First launch** opens a short setup: look, Chrome import, search engine, privacy, memory, AI and sync. Everything can be skipped and changed later in **Settings** (menu ⋯ → Settings, or Ctrl+,).
- **Closing the window** keeps Solus in the notification area (by the clock) so reminders and downloads keep working. To quit completely choose **⋯ → Exit Solus**, or right-click the tray icon → **Exit Solus**.
- **Default browser:** Settings → System → *Make Solus your default browser* (opens Windows' Default apps page).

## Tabs, groups, workspaces, split screen
- **Tab groups:** right-click a tab → *Add tab to group*. Solus also groups tabs automatically when several tabs from one site are open (Settings → Tabs). Click a group's coloured label to collapse it; right-click it to rename, recolour or close it.
- **Workspaces** keep separate sets of tabs (e.g. School, Personal, Research). Use the button at the left of the tab strip, or Ctrl+Shift+1…9.
- **Split screen:** Ctrl+Shift+X, or right-click a tab → *Open in split view*. Drag the divider to resize.
- **Search tabs:** Ctrl+Shift+A. **Reopen closed tab:** Ctrl+Shift+T. **Duplicate tabs** are detected and you're offered to switch.

## Memory and battery
Solus pauses tabs you aren't using and later unloads them completely; they reload when you click them. Choose **Performance**, **Balanced** (default) or **Efficiency** in ⋯ → Performance, or Settings → Performance & memory. On battery Solus switches to Efficiency automatically. **⋯ → More tools → Task manager** (Shift+Esc) shows the memory each tab uses and lets you unload tabs.

## Privacy & security
- Ads and trackers are blocked (Balanced by default). The shield button shows how many were blocked on the page and lets you turn protection off for a site that misbehaves.
- Third-party cookies are blocked; connections are upgraded to HTTPS where possible; Microsoft Defender SmartScreen warns about dangerous sites and downloads.
- Your history is encrypted on this PC. Incognito windows (Ctrl+Shift+N) don't save history.
- Sites ask before using your camera, microphone, location or notifications; change answers from the padlock in the address bar or Settings → Site permissions.
- **⋯ → More tools → Privacy dashboard** shows what's protected. Solus has no telemetry.

## Passwords
Solus offers to save passwords when you sign in. Click the key icon in the address bar to fill a saved password. Showing or exporting passwords asks Windows Hello (PIN, fingerprint or face) to confirm it's you.

**Moving passwords from Chrome:** Chrome only lets its own export read its saved passwords. Settings → Import from Chrome → *Open Chrome's password settings* → **Export passwords**, then *Choose the exported file*. Solus can securely delete the exported file afterwards.

## Notes, tasks and reminders
Open the sidebar (Ctrl+Shift+S): bookmarks, history, downloads, **notes** (attached to a page or a whole site — the note icon in the address bar lights up when a page has notes), **tasks** and **reminders** (shown as Windows notifications, even when the window is closed).

## AI assistant
Ctrl+Shift+E opens the assistant beside the page; Ctrl+Shift+Space opens it as a floating window. It can summarise and explain pages, answer questions about them, compare tabs and organise tabs, bookmarks and workspaces. It needs your own API key — Claude (Anthropic, recommended; pay-as-you-go, separate from a Claude subscription) or Groq (free tier). Settings → AI assistant controls exactly what it may see and do; access to your history is off unless you turn it on, and it asks before closing tabs or deleting bookmarks.

## Sync and backups
Settings → Sync & backups. Choose a folder in your cloud drive (OneDrive, Google Drive, Dropbox) and a passphrase. Use the same folder and passphrase on your other PCs. Data is encrypted before it leaves your PC; **if you forget the passphrase it can't be recovered.** Solus also keeps daily local backups you can restore from the same page.

## Customising
Settings → Appearance for light/dark, themes and density (Compact / Comfortable / Spacious). **⋯ → More tools → Theme editor** to make your own colours or add custom CSS. On the new tab page, the pencil button (top right) customises sections, background, shortcuts and widgets (bookmarks, tasks, notes, weather, links, sticky note, news feed).

## Extensions
⋯ → Extensions. Paste a Chrome Web Store link and choose *Add to Solus*. Many extensions work; some that depend on Chrome-only features won't.

## Keyboard shortcuts
| | |
|---|---|
| New tab / window / Incognito | Ctrl+T / Ctrl+N / Ctrl+Shift+N |
| Close tab / reopen | Ctrl+W / Ctrl+Shift+T |
| Address bar | Ctrl+L (or Alt+D, F6) |
| Find in page | Ctrl+F |
| Bookmark page | Ctrl+D |
| History / downloads | Ctrl+H / Ctrl+J |
| Search tabs | Ctrl+Shift+A |
| Sidebar / AI / floating AI | Ctrl+Shift+S / Ctrl+Shift+E / Ctrl+Shift+Space |
| Split screen | Ctrl+Shift+X |
| Task manager | Shift+Esc |
| Full screen | F11 |

Change them in Settings → Keyboard shortcuts.

## If something goes wrong
- A page that crashes shows a **Reload** button; other tabs keep working.
- If Solus closes unexpectedly, your tabs reopen next time.
- Settings → Sync & backups → *Backups* restores your bookmarks, history, passwords and settings to an earlier day.
- Your data lives in `%LOCALAPPDATA%\Solus\User Data` (Settings → About → Open data folder).
