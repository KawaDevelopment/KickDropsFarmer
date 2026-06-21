# KickDropFarmer

A lightweight, low-resource **Kick.com drops farmer**. Pick a game, and it finds
live drop-enabled streams, registers watch time, tracks real progress, and
auto-claims your drops.

## Download

**[⬇ Get the latest installer from the Releases page](../../releases)** — download

> The installer is unsigned, so Windows SmartScreen may warn you. Click
> **More info → Run anyway**.

## Setup

1. **Launch** KickDropFarmer.
2. **Connect your Kick account.** In your browser, log into kick.com, then:
   - Press **F12** → **Network** tab.
   - Click any request to **`web.kick.com`** (e.g. `drops/campaigns`).
   - Under **Request Headers**, copy the **entire `cookie:` value**.
   - In the app: **Settings → Kick Connection**, paste it, and click **Connect**.
3. **Pick a game** in the **Games** tab (e.g. Rust). The **Home** tab shows it
   farming, with live progress.

Don't have an account? Turn on **Settings → Demo Mode** to explore the whole
UI with simulated data.

## Features

- **Automatic stream discovery** — choose a game; the app finds the live,
  drops-enabled channels for you.
- **Real progress + auto-claim** — reads your actual drop progress from Kick and
  claims rewards the moment they complete.
- **Set-and-forget** — keeps the session alive for long unattended runs, fails
  over automatically when a stream goes offline.
- **Two themes** — a retro-futuristic look and a clean
  *Minimalist* "design".
- **6 languages** — English, 日本語, Español, Français, 中文, 한국어.
- Start-on-boot, system tray, scheduler (farm only within set hours), and
  desktop + Discord notifications — all optional.

## Building from source (nerd)

Requires **Rust**, **Node.js**, and — because the HTTP client impersonates a real
browser's TLS fingerprint (BoringSSL) — **NASM** and **libclang**:

- Install **NASM** and put it on your `PATH`.
- Install **libclang** (e.g. `pip install libclang`) and set the `LIBCLANG_PATH`
  environment variable to its `clang/native` directory.

Then:

```bash
npm install
npm run tauri build     # production installer (NSIS)
npm run tauri dev       # dev build with a live window
```

The installer is written to `src-tauri/target/release/bundle/nsis/`.

## How it works

Kick has no official drops API, so the app talks to Kick's web API through a
TLS-impersonating client (to clear Cloudflare), authenticated with your pasted
session cookies. It opens a lightweight viewer websocket for the stream it's
watching to register watch time, reads progress from `/drops/progress`, and posts
claims. Kick credits one stream at a time, so it farms a single stream only.

## Notes & disclaimer

- **Unofficial tool.** It can break whenever Kick changes its site or APIs.
- **Farming drops likely violates Kick's Terms of Service — use at your own risk.**
- Your Kick session cookies are stored **locally only**, and used solely to talk
  to Kick's API on your behalf.

- **KAWA DEVELOPMENT DOES NOT TAKE RESPONSIBILITY FOR ANY ACCOUNT OR ACCOUNTS**
  **LOST, DAMAGED, BANNED, OR OTHERWISE INACCESSIBLE DUE TO USAGE OF THIS APPLICATION.**
