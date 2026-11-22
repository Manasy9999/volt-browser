# Volt benchmarks: memory, CPU and tab-switch latency

This page gives the full method and results behind the numbers in the [README](../README.md). Every figure was measured on one PC; nothing here is extrapolated. If you rerun these measurements and get different numbers, please open an issue with your method and results.

## Method

Measurements taken on 22 November 2026 on a single PC: AMD Ryzen 9 7900X, 64 GB RAM, Windows 11.

Versions: Volt 0.1.0 (WebView2 engine 157.0.4341.60), Google Chrome 149.0.7827.114, Microsoft Edge 158.0.4370.33, Opera GX 139.0.6053.63 (Chromium 155).

Each browser is launched on its own, with a fresh profile, no extensions, its default settings, sound muted, in a normal window that is neither minimized nor hidden. It is given the same list of 50 addresses spread over about fifteen sites: YouTube, Wikipedia, GitHub, Reddit, Twitch, Amazon, Stack Overflow, MDN, Le Monde, Le Figaro, BBC, docs.rs, Hacker News, X, Discord, Steam. For Volt, the 50 addresses are supplied as a session to restore.

Three minutes after launch, the following are recorded over the browser's whole process tree:

- **Commit**: the sum of the committed private memory of each process.
- **RAM in use**: the sum of resident private pages.
- The number of processes.
- Dedicated video memory.

Three configurations were repeated (Volt default with the ad blocker off, Chrome, and Volt with all 50 tabs loaded): the difference from one run to the next stayed under 4 % for commit.

## Results: 50 tabs

| Browser                                                         | Tabs loaded | Commit           | RAM in use       |
| --------------------------------------------------------------- | ----------: | ---------------: | ---------------: |
| Volt 0.1.0 (WebView2 157), defaults, ad blocker off             | 7 of 50     | 1,104–1,147 MB   | 842–885 MB       |
| Volt, full defaults (ad blocker on)                             | 7 of 50     | 1,253 MB         | 965 MB           |
| Google Chrome 149                                               | 50          | 4,610–4,638 MB   | 3,606–3,686 MB   |
| Microsoft Edge 158 (signed in, sync on — see limitations)       | 50          | 5,066 MB         | 3,971 MB         |
| Opera GX (Chromium 155)                                         | 50          | 5,402 MB         | 4,426 MB         |
| Volt, all 50 tabs kept loaded                                   | 50          | 5,027–5,058 MB   | 1,359–1,435 MB   |

What these results show, and what they do not:

- The gain comes only from tab management; the engine is the same Chromium.
- With all 50 tabs kept loaded, Volt commits about 9 % more than Chrome, and uses about 950 MB of video memory, 5 to 10 times more than the other browsers. With 7 tabs loaded, Volt's video memory is 130 MB.
- The built-in ad blocker (uBlock Origin Lite) adds about 130 MB in this scenario.

## Other measurements

Same machine.

| Scenario                                          | Result                                              |
| ------------------------------------------------- | --------------------------------------------------- |
| 100 tabs open, 7 loaded: commit                   | about 1.0 GB                                        |
| 100 tabs open, 7 loaded: RAM in use               | 628 MB with the window open, 32 MB minimized        |
| CPU at idle                                       | 0.9 % of one core                                   |
| CPU at idle, all hidden tabs frozen               | 0.03 % of one core                                  |
| Switching to a loaded or frozen tab               | 7 to 21 ms                                          |
| Switching to an unloaded tab                      | 0.36 to 0.82 s (medium to heavy page)               |
| Opening the bookmarks page with 136 bookmarks     | 5 to 18 ms                                          |

## Limitations

One scenario, on one machine.

Chrome, Edge and Opera GX keep all 50 tabs loaded, whereas Volt by default keeps only 7; it unloads the others, which reload in about half a second when clicked, and that cost is not measured in the table.

In this test, the 7 tabs Volt keeps loaded are the first 7 in the list, all on YouTube; with a different mix of sites the figure would be different.

The other browsers have their own memory saver, which does not kick in within 3 minutes.

With a fresh profile, some pages show a consent request or an anti-bot check, the same way for every browser.

Edge automatically signed in to the Microsoft account of the Windows session and turned on sync: its measurement includes this.

Opera GX also opens its "GX Corner" page.
