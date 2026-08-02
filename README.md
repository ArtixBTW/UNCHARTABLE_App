# UNCHARTABLE

A chart manager for **UNBEATABLE**.

UNCHARTABLE is a lightweight Windows/Linux app for discovering, previewing, installing,
updating, and managing community charts from
[unchartable.site](https://unchartable.site).

![UNCHARTABLE](public/unchartable.png)

## What it does

- Browses the complete UNCHARTABLE chart catalog.
- Searches and filters charts by title, artist, charter, difficulty, and rank.
- Plays chart previews directly in the app.
- Installs charts safely into UNBEATABLE's `CustomSongs` directory.
- Detects compatible charts that were installed manually.
- Keeps supported charts updated without replacing a working installation first.
- Lets players restore or permanently remove charts from the app's local trash.
- Opens UNBEATABLE directly through Steam.

UNCHARTABLE is portable, account-free, and available natively for Windows and Linux. Linux
builds detect common Steam, Flatpak Steam, and additional Steam library locations, so Bottles
is not required. The app does not run a background service or require administrator access.

## Safety

Chart archives are treated as untrusted input. The native installer validates
download size, archive paths, extracted files, and chart contents before changing
the game library. Managed charts include local metadata so updates and removal
only affect the intended chart.

Official builds are published in
[Releases](https://github.com/ddecry/UNCHARTABLE_App/releases) with a SHA-256
checksum. See [SECURITY.md](SECURITY.md) for the security model and reporting
instructions.

## Privacy

UNCHARTABLE does not require an account and does not include analytics,
advertising, or telemetry. It connects to `https://unchartable.site` to load the
chart catalog, artwork, previews, metadata, and chart archives. It opens Steam
only when the player explicitly asks it to launch UNBEATABLE.

The selected game directory, theme, update preference, and chart-management
metadata are stored locally on the player's computer. UNCHARTABLE does not upload
the contents of the player's `CustomSongs` directory.

## Project

UNCHARTABLE uses React and TypeScript for the interface and Rust with Tauri for
the native operating system layer.

```text
src/                  desktop interface
src-tauri/src/        catalog, validation, installation, and library management
src-tauri/icons/      application icons
.github/workflows/    verification and release builds
```

## Bug reports

Use **report bug** in the app settings to open a prefilled GitHub issue with the app version
and operating system. Reports are public, so do not attach private charts or personal paths.

## License

The source code is available under the [MIT License](LICENSE). UNCHARTABLE and
game-related visual assets remain subject to their respective owners' rights.

UNCHARTABLE is a community project and is not affiliated with or endorsed by
D-CELL GAMES.
