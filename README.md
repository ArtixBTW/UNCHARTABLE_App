# UNCHARTABLE

The official UNCHARTABLE chart manager for **UNBEATABLE**.

UNCHARTABLE is a lightweight Windows app for discovering, previewing, installing,
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

UNCHARTABLE is portable, account-free, and focused on Windows. It does not run a
background service or require administrator access.

## Safety

Chart archives are treated as untrusted input. The native installer validates
download size, archive paths, extracted files, and chart contents before changing
the game library. Managed charts include local metadata so updates and removal
only affect the intended chart.

Official builds are published in
[Releases](https://github.com/ddecry/UNCHARTABLE_App/releases) with a SHA-256
checksum. See [SECURITY.md](SECURITY.md) for the security model and reporting
instructions.

## Project

UNCHARTABLE uses React and TypeScript for the interface and Rust with Tauri for
the native Windows layer.

```text
src/                  desktop interface
src-tauri/src/        catalog, validation, installation, and library management
src-tauri/icons/      application icons
.github/workflows/    verification and release builds
```

Useful development commands:

```powershell
npm install
npm run check
npm run tauri dev
npm run build:portable
```

## License

The source code is available under the [MIT License](LICENSE). UNCHARTABLE and
game-related visual assets remain subject to their respective owners' rights.

UNCHARTABLE is a community project and is not affiliated with or endorsed by
D-CELL GAMES.
