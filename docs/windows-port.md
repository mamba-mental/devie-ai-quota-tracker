# Windows 11 port

This fork adapts Devie AI Quota Tracker 0.14.0 for Windows 11 x64.
Upstream baseline: `9f2ea2542f0415b186dd58d300721654d6b52eee`.

## Delivery checklist

- [x] Dedicated dated project directory and `windows-11` branch.
- [x] Personal GitHub fork and remote `windows-11` branch created.
- [x] Windows build, tray integration, and protected credential storage.
- [x] Automated checks and a Windows installer.
- [x] Installed application smoke test and account preservation check.

## Platform choices

- The Windows app identifier is `com.primemind.deviequota.windows`, separate
  from the upstream Mac app. Tauri chooses the Windows application data path.
- The system tray shows the selected remaining quota as a number. Hover for
  the account/window summary; click for the quota popup; right-click for the menu.
- Windows credentials use current-user DPAPI encryption. They can be decrypted
  by processes running as that Windows user, not by another Windows login.
- The Windows installer is per-user NSIS. It does not require administrator access.
- Windows updates are installed manually. The upstream Mac updater feed is not
  used for this fork; no upstream signing credentials are required or copied.
- Existing providers use browser sign-in. OpenCode Go / Go Plus uses a masked
  API-key field. Existing CLI credentials are not imported.
- A second Windows launch brings the existing window forward instead of
  starting a duplicate background process.

## OpenCode Go / Go Plus

Open **Settings > OpenCode Go > Connect account** and enter a key from your
subscribed OpenCode workspace. The app reads the official
`GET https://opencode.ai/zen/go/v1/usage` endpoint with Bearer authentication.
It displays the reported rolling, weekly, and monthly percentages and reset
times. It does not send model inference requests or infer an account balance,
account identity, or subscription tier that the endpoint does not report.

Keys are validated before saving, encrypted with the same Windows account
protection as other logins, and excluded from account labels and telemetry.
Replacing a key on an existing connection preserves its ID and preferences.
Because the endpoint does not return account identity, adding the same key as a
new connection creates a separate entry. New connection strings currently use
English text in all locale files; complete new-string translations remain open.

## Windows verification (2026-10-07)

- Frontend formatting/lint and TypeScript checks passed.
- Translation-key checks and production frontend build passed.
- Rust formatting passed; all 91 Rust tests passed, including Windows
  credential encryption, tamper rejection, tray icon pixels, and OpenCode parsing
  and reconnection tests.
- The x64 NSIS installer built successfully and installed without elevation.
- The installed native window, provider settings, manual-update message,
  close/reopen behavior, and single-instance behavior were exercised.
- Four user-connected accounts (two Claude and two Codex) retained their IDs
  and reported Ready after recovery into the normal Windows profile.
- A user-entered OpenCode key connected successfully and returned all three
  live quota windows. All five accounts retained their IDs and reported Ready
  after a full exit and fresh launch through the repaired desktop shortcut.
- Other provider services, native notification delivery, and autostart across a
  Windows sign-in have not been independently verified.

### Installation from a packaged automation host

Run the installer from Windows Explorer or an ordinary Windows terminal.
Launching it from a packaged host such as the Microsoft Store Codex app can
redirect installation and account storage into that host's private `LocalCache`
folder. Launching the desktop shortcut later can then display a separate empty
account store. This was reproduced during installation and corrected by backing
up both stores, restoring the original four accounts into the normal Windows
profile, installing outside the packaged host, and replacing the shortcut.

The intended per-user executable is
`%LOCALAPPDATA%\Devie AI Quota Tracker\devie-quota.exe` and account storage is
`%APPDATA%\com.primemind.deviequota.windows`. Do not overwrite either account
store if it already contains accounts; compare and preserve both first.

## Build

Install Bun, stable Rust, and Visual Studio C++ Build Tools, then run from the
project directory in PowerShell:

```powershell
pwsh -File scripts/build-windows.ps1
```

The equivalent individual build commands are:

```powershell
bun install --frozen-lockfile
bun run check
bun run i18n:verify
bun run build:desktop-frontend
cargo test --locked --manifest-path src-desktop/Cargo.toml
bun run tauri build --bundles nsis -- --locked
```

Windows configuration is selected automatically by Tauri from
`src-desktop/tauri.windows.conf.json`. The installer is generated under
`src-desktop/target/release/bundle/nsis/`.

The local installer is unsigned. Windows publisher signing and a signed update
feed are separate distribution tasks; they are not claimed by this local build.
