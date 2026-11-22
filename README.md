<p align="center">
  <img src="logos/volt.png" width="112" alt="Volty, the Volt mascot: a purple robot head with a lightning-bolt antenna">
</p>

<h1 align="center">Volt — a lightweight, low-memory web browser for Windows, written in Rust</h1>

<p align="center">
  <strong>Chromium compatibility through Microsoft WebView2. With 50 tabs open, it uses just over a quarter of the memory Chrome uses, because it keeps only 7 of them loaded.</strong>
</p>

<p align="center">
  <a href="README.fr.md">Français</a> ·
  <a href="docs/BENCHMARKS.md">Benchmarks</a> ·
  <a href="#limitations">Limitations</a> ·
  <a href="#build-from-source">Build</a>
</p>

<p align="center">
  <img src="docs/img/hero.png" width="860" alt="The Volt window showing its bookmarks page: bookmarks laid out as tiles, sorted into folders">
</p>

Volt is a lightweight browser and Chrome alternative for people who want a low RAM browser without giving up site compatibility. It is a small Rust application (built on the `tao` and `wry` libraries) around the Chromium engine that Microsoft ships with Windows, WebView2. It is not a new engine: pages render exactly as they do in Chromium. The savings come from how Volt manages tabs, not from the engine.

- One executable of about 1 MB, no installer.
- Needs WebView2, which is already part of Windows 11.
- Version 0.1.0, developed by one person. The interface is in French only for now.

## Memory usage compared with Chrome, Edge and Opera GX

50 identical tabs across about fifteen sites, fresh profiles, default settings, no extensions, measured 3 minutes after launch over the browser's whole process tree. One PC: AMD Ryzen 9 7900X, 64 GB RAM, Windows 11, 22 November 2026.

| Browser                                                         | Tabs loaded | Commit           | RAM in use       |
| --------------------------------------------------------------- | ----------: | ---------------: | ---------------: |
| Volt 0.1.0 (WebView2 157), defaults, ad blocker off             | 7 of 50     | 1,104–1,147 MB   | 842–885 MB       |
| Volt, full defaults (ad blocker on)                             | 7 of 50     | 1,253 MB         | 965 MB           |
| Google Chrome 149                                               | 50          | 4,610–4,638 MB   | 3,606–3,686 MB   |
| Microsoft Edge 158 (signed in, sync on — see limitations)       | 50          | 5,066 MB         | 3,971 MB         |
| Opera GX (Chromium 155)                                         | 50          | 5,402 MB         | 4,426 MB         |
| Volt, all 50 tabs kept loaded                                   | 50          | 5,027–5,058 MB   | 1,359–1,435 MB   |

Read this table carefully:

- The difference comes only from tab management. Chrome, Edge and Opera GX keep all 50 tabs loaded; Volt keeps 7 and unloads the rest. Clicking an unloaded tab reloads it, which takes 0.36 to 0.82 s on a medium to heavy page. That cost is not in the table.
- With all 50 tabs kept loaded, Volt commits about 9 % more memory than Chrome and uses about 950 MB of video memory, 5 to 10 times more than the other browsers (130 MB with 7 tabs loaded).
- The built-in ad blocker adds about 130 MB in this scenario.
- Two identical runs differed by less than 4 % on commit.
- One scenario, one machine. In this test the 7 tabs Volt kept loaded were all YouTube; another mix of sites would give a different number.

Full method, versions and every limitation of the test: [docs/BENCHMARKS.md](docs/BENCHMARKS.md).

Other measurements on the same machine:

| Scenario                                          | Result                                         |
| ------------------------------------------------- | ---------------------------------------------- |
| 100 tabs open, 7 loaded                           | about 1.0 GB commit                            |
| Same, RAM in use                                  | 628 MB window open, 32 MB window minimized     |
| CPU at idle                                       | 0.9 % of one core (0.03 % with hidden tabs frozen) |
| Switching to a loaded or frozen tab               | 7 to 21 ms                                     |
| Switching to an unloaded tab                      | 0.36 to 0.82 s (medium to heavy page)          |
| Opening the bookmarks page with 136 bookmarks     | 5 to 18 ms                                     |

## How tab suspension works

- **Loaded-tab budget.** By default Volt keeps 7 tabs loaded (adjustable from 3 to 25, or unlimited). Other tabs are unloaded and reload when you click them. The active tab, tabs playing audio, tabs where you are typing text and pinned tabs are never unloaded.
- **Freezing.** A hidden tab is frozen after 2 minutes; the engine then moves its memory out of RAM.
- **Coming back to an unloaded tab.** A screenshot of the page appears first, then the reloaded page, with scroll position and video position restored.
- **The trade-off.** Anything that existed only inside the page is lost when it unloads: unsent input outside a text field, the state of a web app. Pin a tab, or raise or remove the budget, to keep it loaded.
- **Nothing runs at idle.** The interface's permanent animations are off by default.

<p align="center">
  <img src="docs/img/tabs.png" width="860" alt="Volt's tab bar with pinned tabs and unloaded tabs">
</p>

## Features

**Tabs and sessions**
- Pinned tabs.
- Tab right-click menu: new tab to the right, duplicate, pin, mute, close other tabs, close tabs to the right, reopen closed tab (Ctrl+Shift+T).
- Offers to restore the last session at startup.

**Bookmarks and history**
- Bookmarks bar in three styles, and a bookmarks page laid out as tiles.
- Import bookmarks from Chrome, Edge, Brave, Vivaldi, Opera and Opera GX.
- Automatic sorting of bookmarks into categories, computed on your machine; nothing is sent anywhere.
- Side panel with history grouped by day, and per-site cookie deletion.

**Chrome extensions and blocking**
- Install Chrome extensions from the Chrome Web Store by their address, or with one click on their store page. uBlock Origin Lite and SponsorBlock are recommended.
- Built-in ad blocker (uBlock Origin Lite).

**Cookie banners and translation**
- Automatic answers to cookie banners, off by default: refuse all or accept all, with a list of excluded sites you fill in yourself. Measured on 25 sites: in refuse mode every site that offers a free refusal was handled and no consent was recorded by mistake. Sites that only offer "accept or subscribe" keep their banner in refuse mode.
- Translate-page button in the address bar, through Google Translate, with a "show original" button on a translated page. Automatic translation of foreign-language pages is an option, off by default.

**Everything else**
- Update check from the settings: the new executable is downloaded from this repository's releases, checked against its published SHA-256, and Volt restarts with your session restored.
- Passwords saved and filled in by the engine, with a button to delete the saved passwords.
- Game Mode, for a gaming browser that stays out of the way: low priority and fast tab freezing while you play.
- Six themes.
- Can be set as the default browser.

<p align="center">
  <img src="docs/img/favbar.png" width="860" alt="The Volt bookmarks bar under the address bar">
</p>

<p align="center">
  <img src="docs/img/history.png" width="420" alt="The history side panel: visits grouped by day, with a button to delete a site's cookies">
  <img src="docs/img/settings.png" width="420" alt="The Volt settings page">
</p>

## Limitations

- Windows 10 and 11 only.
- Interface in French only.
- Uses Microsoft's Chromium engine (WebView2); it is not its own engine.
- An unloaded tab reloads and loses state that existed only in the page.
- Extension popups cannot be shown (the engine does not allow it); only an extension's options page opens.
- No list where you can view your saved passwords.
- The executable is not signed: Windows SmartScreen or Defender may show a warning.
- High video memory use when you keep many tabs loaded.
- Page translation goes through Google's relay: the page address and content are sent to Google, you are not signed in on the translated page, forms are blocked there, and heavy web apps may not display.
- Updates rely on GitHub over HTTPS and a published checksum; there is no code signature.
- Young project, no security audit.

## Try it

Download `volt.exe` from the [Releases](../../releases) page and run it. No installation is needed. On Windows 10, install the Microsoft Edge WebView2 Runtime if it is not already present.

## Build from source

Requirements: Windows 10 or 11, the stable Rust toolchain (the crate uses edition 2024, so Rust 1.85 or later).

```
cargo build --release
```

The executable is written to `target\release\volt.exe`. Close Volt before rebuilding: Windows will not let the build replace a running executable.

## License and legal status

There is no open-source license for now. The code is published so you can read it, build it and try it. All rights reserved: ask before reusing or redistributing it.

Volt is not affiliated with Google or Microsoft. Chrome, Chromium, Edge and WebView2 are trademarks of their respective owners.
