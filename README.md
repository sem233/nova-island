<div align="center">

# Nova Island

**A living island for your Windows desktop.**

Your music, apps, files and alerts in one glass capsule at the top of your screen — with Nova, a little companion who keeps you company.

[**Download for Windows**](https://nova-tech-kit.github.io/nova-island/) · [Microsoft Store](https://apps.microsoft.com/detail/9PHV3N1V2H7M) · Free and open source (GPL v3)

[![Latest release](https://img.shields.io/github/v/release/NOVA-TECH-KIT/nova-island?style=for-the-badge&label=Version&color=3b82f6&labelColor=1c1a2e)](../../releases/latest)
[![Stars](https://img.shields.io/github/stars/NOVA-TECH-KIT/nova-island?style=for-the-badge&label=Stars&color=f472b6&labelColor=1c1a2e&logo=github&logoColor=white)](../../stargazers)
[![Support on Ko-fi](https://img.shields.io/badge/Support-Ko--fi-ff5e5b?style=for-the-badge&labelColor=1c1a2e&logo=kofi&logoColor=white)](https://ko-fi.com/binamer)
[![Join on Telegram](https://img.shields.io/badge/Join-Telegram-2aabee?style=for-the-badge&labelColor=1c1a2e&logo=telegram&logoColor=white)](https://t.me/nova4apps)

![Meet Nova Island](promo/en/01-hero.png)

</div>

## Features

### Say hi to Nova
A little companion who lives in your island. Nova keeps you company while you focus, welcomes you back, cheers your wins, and reminds you to drink water and rest your eyes.

![Nova](promo/en/02-nova.png)

### Keeps playing while you browse
Title, artwork and live waveforms of whatever is playing, plus a floating mini player in one click.

![Browser](promo/en/03-browser.png)

### Never miss a prayer
Prayer times with a heads-up before the adhan, plus dhikr, du'a and Qur'an verses in the KFGQPC Hafs script, as often as you choose.

![Prayer](promo/en/06-faith.png)

### Downloads, at a glance
Progress, speed and time left while you work, with IDM Nova — pause, resume or open a file in one click.

![Downloads](promo/en/04-downloads.png)

### Make it yours
13 free themes in Settings → Themes, from liquid glass to pure black, sakura pink and pixel green. None of them makes Nova heavier, and the solid ones are even lighter.

![Themes](promo/en/08-themes.png)

### Smart about what you do
Copy a link, a sum or a color and the island knows what to do with it. It also lets you know when an app turns on your mic or camera.

![Smart](promo/en/05-smart.png)

### Your island, your way
Turn on only what you need from one simple settings window. Vertical on the screen edge, hides in full screen, starts with Windows.

![Settings](promo/en/07-settings.png)

## Install

1. Download **Nova Island** from [the website](https://nova-tech-kit.github.io/nova-island/), or get it from the [Microsoft Store](https://apps.microsoft.com/detail/9PHV3N1V2H7M).
2. Run it. With the direct download, Windows may show "Windows protected your PC" because the app is not code-signed yet — choose **More info → Run anyway**. The Store version doesn't show it.
3. The island appears at the top of your screen. Press **Ctrl + Alt + I** to open it.

Requires Windows 10 or 11 (64-bit).

## What's new in 1.8.12

**Drastically lower idle RAM & CPU** — switches WebView2 to `MemoryUsageTargetLevel::LOW` whenever the island is resting collapsed and restores `NORMAL` on hover/pin/interaction, suspends WebView2 (`TrySuspend`) and pauses CSS animations when the island is hidden, deduplicates `media` IPC updates, slows idle stats/layout/reminder polling, and guards Prayer DOM updates so all animations and features stay identical while cutting idle memory and CPU usage dramatically.

## What's new in 1.8.11

**Sleeker browser-tab indicator ("Thin line")** — redesigned the `#island.addr-line` browser-tab micro-notch (`96×6 px`) across all themes so theme decorations (`--th-deco`), update dots, cat ears, and blur backdrop artifacts no longer bleed across Chrome/Edge tab headers; replaces the harsh striped bar with a clean top-docked capsule and centered theme-accent handle that expands smoothly into the full island on hover or click.

**Clean release artifact staging** — cleared cached installer executables in the GitHub Actions release workflow before building NSIS packages so only the active release's setup, portable executable, and `latest.yml` manifest are published.

## What's new in 1.8.10

**Island Calculator add-on** — click the new calculator icon next to News/Settings in the expanded topbar to open a compact DPI-scaled calculator (`+`, `−`, `×`, `÷`, decimals) with live preview, standard operator precedence, Arabic-Indic digit normalization (`٠-٩`, `٫`), LTR display/grid preservation in Arabic UI, hold-to-repeat backspace, full theme & blur/solid/no-animation support, and an on/off toggle under **Settings › Island › Tools › Calculator**.

**Dual-source update check (`sem233/nova-island` + official)** — kept **Check for updates** pointing to the upstream `NOVA-TECH-KIT/nova-island` repository, added a dedicated **Check fork updates (`sem233/nova-island`)** button in **Settings › General**, updated the app updater version baseline to `1.8.10`, and added `latest.yml` manifest generation + upload to the release workflow so in-app fork updates download and install automatically.

## What's new in 1.8.9

**Reliable SPA & HLS video extraction** — added an audio-muted WebView2 runtime stream sniffer fallback for JavaScript-rendered video pages (such as `cinejoy.pk`), separated `sourcePage` / `Referer` / `Origin` headers for both probing and downloading via `yt-dlp`, upgraded generic stream filenames (e.g. `master`) to clean page titles, restored strict TLS certificate verification, and surfaced specific error messages for TLS, DRM, login walls, anti-bot gates, and unsupported links.

**Clipboard scrolling in System** — fixed flex/grid overflow sizing, wheel-event routing, and drag exclusions so scrolling inside both `#clip-list` and long `.clip-full-text` snippets works smoothly without changing system volume or dragging the island window.

**Overlay click-through & dim fixes** — fixed stuck dimming (`nova-dim`) and unclickable states across `ghost`, `clickable`, `pinned`, and `toggle` transitions, cleared stale browser tab intersections immediately on window switch, prevented expanded panel geometry from overwriting collapsed `rest_rect`, and raised overlay dim opacity cleanly while preserving intended click-through over browser tabs.

**UTF-8 QuickClips & test reliability** — kept UTF-8-safe clip truncation (`clips.rs`) and made calendar parity tests deterministic across CI timezones (`cargo test --workspace --locked`).

## What's new in 1.8.8

**Video downloader (MDM)** — download any video or high-quality MP3 stream from the browser in one click. Hover the download button to pre-fetch formats instantly; pick 4K, 1080p, 720p, 480p, 360p or MP3 320 k from the quality drawer. Requires [yt-dlp](https://github.com/yt-dlp/yt-dlp) and [ffmpeg](https://ffmpeg.org/) in PATH or beside the app.

**QuickClips manager** — save up to 200 text snippets, pin the ones you use most, copy any clip back to the clipboard in one click, and add from clipboard with the paste button. Clips are stored locally (`userData/clips.json`); nothing leaves your machine.

**IDM Nova downloads** — progress, speed, time left and ETA for each active IDM Nova download shown directly on the island; pause, resume, cancel or open/reveal a file without switching windows.

**Smart clipboard** — copy a URL, a math expression (e.g. `12 * 4 + 8`) or a hex/RGB color and the island shows the right action: open/download the link, compute the result with one tap to copy it, or preview the color swatch.

## Build from source

Nova Island is written in Rust; the app is in [`nova-next/`](nova-next/). You need Windows 10/11 and
[Rust](https://rustup.rs) (MSVC toolchain):

```bash
cd nova-next
cargo build --release -p nova-app     # target/release/nova-island.exe
```

Everything else (how the pages and the app fit together, tests, the installers) is in
[nova-next/README.md](nova-next/README.md). Want to help? Read [CONTRIBUTING.md](CONTRIBUTING.md) first.

## Support Nova Island ☕

Nova Island is free, with no ads and no tracking, and it always will be. If it makes your day a little better, you can buy us a coffee:

- **Ko-fi** (cards, PayPal): [ko-fi.com/binamer](https://ko-fi.com/binamer)
- **InstaPay** (Egypt): [Send via InstaPay](https://ipn.eg/S/binamerbm/instapay/38txJO)

Stars ⭐ and sharing it with a friend help a lot too.

## License

Nova Island is free software, licensed under the [GNU General Public License v3.0](LICENSE).

Copyright © 2026 Nova Island by AmerTech.

The Qur'an font in `fonts/` (KFGQPC HAFS Uthmanic Script by the King Fahd Glorious Quran Printing Complex) is included unmodified under its own license — see [fonts/KFGQPC-LICENSE.txt](fonts/KFGQPC-LICENSE.txt). It is not covered by the GPL. The Aref Ruqaa font in `nova-next/assets/fonts/` is under the SIL Open Font License — see [OFL.txt](nova-next/assets/fonts/OFL.txt).

The name "Nova Island" and its logo are not covered by the GPL; please use a different name for modified versions.
