# LAV — Tauri Build Instructions

## Prerequisites (one-time setup)

1. **Install Rust** — https://rustup.rs/
   ```
   # Run the installer, accept defaults
   rustup update stable
   ```

2. **Install Tauri CLI**
   ```
   cargo install tauri-cli
   ```

3. **WebView2** — Usually already installed on Windows 10/11.
   If not: https://developer.microsoft.com/en-us/microsoft-edge/webview2/

## Build

From `C:\Users\hrist\lol-account-manager-tauri\`:

```
cargo tauri build
```

The first build takes 5–15 minutes (compiling Rust + all crates).
Subsequent builds are much faster (incremental).

**Output:**
- `src-tauri\target\release\bundle\nsis\LAV_1.0.0_x64-setup.exe`  ← Installer to distribute
- `src-tauri\target\release\lol-account-manager.exe`               ← Raw binary

## Dev mode (with hot-reload of frontend)

```
cargo tauri dev
```

## Why this is better than the Electron version

| | Electron | Tauri |
|---|---|---|
| Installer size | ~400 MB (includes Chromium) | ~12 MB |
| Chromium needed | Yes (bundled) | No (uses system WebView2) |
| op.gg scraping | Puppeteer (full browser) | Plain HTTP + HTML parsing |
| Runtime | Node.js | Native Rust |

## Vault compatibility

The encrypted vault file (`AppData\Roaming\LAV\accounts.enc`) uses the same format
as the original Electron app. Existing accounts are preserved — no migration needed.
