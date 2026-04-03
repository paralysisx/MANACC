# VaultX

A lightweight Windows desktop app for managing multiple League of Legends accounts in one place. Built with Tauri v2 and Rust.

## Features

- **Encrypted Vault** — All credentials are stored locally and protected behind a master password. Nothing is sent to any server.
- **Account Cards** — Each account displays its Riot ID, region, rank (Solo/Duo & Flex), win rate, wins/losses, and top champions pulled live from op.gg.
- **Lobby Viewer** — Reveals the ranks and profiles of everyone in your current lobby before the game starts.
- **Auto Accept** — Automatically accepts the match-found screen in the background.
- **Refresh Stats** — Update a single account or all accounts at once with the latest ranked data.
- **Sort Accounts** — Sort your accounts by highest rank, lowest rank, or region.
- **Copy Credentials** — Copy your username or password to clipboard directly from the card.
- **Color Themes** — Switch between Default (purple), Dark, Light, Galaxy, and Starry Night themes from settings.
- **Auto Updater** — Built-in update checker pulls new releases directly from GitHub.
- **Launch on Startup** — Optional Windows startup entry so VaultX is always ready.

## How It Works

1. On first launch you set a master password that encrypts your vault.
2. Add your League accounts (username, password, Riot ID, region).
3. Stats are fetched automatically from op.gg — no API key required.
4. Click the play button on any card to log in and launch the client.

## Stack

- **Frontend** — Vanilla JS, HTML, CSS (no framework)
- **Backend** — Rust via Tauri v2
- **Stats** — Scraped from op.gg
- **Storage** — Local encrypted file (AES)
- **Updater** — Tauri updater plugin via GitHub Releases

## Installation

Download the latest `.exe` installer from the [Releases](../../releases) page and run it.

> Windows only. Requires League of Legends to be installed.

## Building from Source

```bash
npm install
npm run tauri build
Requires Rust and the Tauri CLI.

Disclaimer
This app stores credentials locally on your machine only. Use at your own discretion and in accordance with Riot Games' Terms of Service.
