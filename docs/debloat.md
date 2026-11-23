# Debloat: research and status

As of 23 November 2026. The app list in `src/apps.rs` was compared with:

- [Win11Debloat](https://github.com/Raphire/Win11Debloat) (`Config/Apps.json`, 162 apps, plus `Features.json` and the reg files)
- [WinUtil](https://github.com/ChrisTitusTech/winutil) (`config/tweaks.json`)
- [Sophia Script](https://github.com/farag2/Sophia-Script-for-Windows)
- [tiny11builder](https://github.com/ntdevlabs/tiny11builder) (`CleanupCatalog.cs`)
- two Windows 10 debloaters, [Sycnex](https://github.com/Sycnex/Windows10Debloater) and [W4RH4WK](https://github.com/W4RH4WK/Debloat-Windows-10)
- Microsoft's documentation:
  - [Provisioned apps in Windows 10 1709](https://learn.microsoft.com/en-us/windows/application-management/remove-provisioned-apps-during-update)
  - [Policy CSP ApplicationManagement](https://learn.microsoft.com/en-us/windows/client-management/mdm/policy-csp-applicationmanagement)
  - [Services in Windows Server](https://learn.microsoft.com/en-us/windows-server/security/windows-services/security-guidelines-for-disabling-system-services-in-windows-server). The page was cut off, so its later rows weren't checked.
- [file.net](https://www.file.net/process/mcafee-security.exe.html), for the package name of McAfee's Store app

The installed packages and the services were also checked on a Windows 11 Pro 26H2 PC.

## Built in (no administrator rights)

### New in `CATALOG`

**Retired or hardly used (recommended)**
- Translator and Power BI.
- The Remote Desktop app from the Store. Remote Desktop Connection (`mstsc`) stays.
- AI Hub.
- PC Manager, under both of its package names.

**Widgets (recommended)**
- Nobody uses them.
- `MicrosoftWindows.Client.WebExperience` and `Microsoft.StartExperiencesApp` are both named "Widgets", so they are removed together.
- `Microsoft.WidgetsPlatformRuntime` stays protected.

**Left over from Windows 10 (recommended)**
- Windows Feedback, Office Lens, Fresh Paint, Phone, Phone Companion, Microsoft Wi-Fi and Reading List.

**Trials, promotions and games (recommended)**
- McAfee's Store app (`5A894077.McAfeeSecurity`).
- Whole publishers by prefix:
  - `flaregamesGmbH.`, which replaces the single entry for Royal Revolt 2
  - `GAMELOFTSA.` (Asphalt)
  - `D52A8D61.` (Zynga)
- Photo, drawing and media apps: NYT Crossword, Photoshop Express, PicsArt, Polarr, SketchBook, Phototastic Collage, WinZip Universal and CyberLink Media Suite.
- Other promotions: Flipboard, Duolingo, XING, Shazam, iHeartRadio, TuneIn, Yandex Music, Twitter, Wunderlist and Fitbit Coach.

**PC makers' welcome and registration apps (recommended)**
- HP JumpStarts, HP Welcome, HP Registration, HP Connected Music and HP Connected Photo.
- Acer Registration.
- Samsung Welcome.

**Offered, but never picked for you**
- Microsoft: Family, Journal, Whiteboard, Windows App and Microsoft 365 Companions.
- Streaming: Netflix, Prime Video, Disney+, Hulu, Sling TV, Plex and Pandora.
- Social: TikTok, and every app from `Facebook.` (Facebook, Instagram, Messenger). WhatsApp is published as `5319275A.`, so it isn't affected.
- Other apps: Amazon, Keeper and Drawboard PDF.
- Second packages of apps already listed, which are shown as one app with them:
  - `Microsoft.Office.Todo.List`, To Do's old name
  - `Microsoft.XboxGameOverlay`, Game Bar's old plugin

**How the list matches packages**
- Prefixes are now compared regardless of case, because publishers write their names both ways, such as `Facebook.` and `FACEBOOK.`.
- A test checks that packages with the same name agree on being recommended and on the note. If they didn't, the list wouldn't merge them.

### New in `TRIALS` (step-by-step guide for Settings)

- CyberLink and WPS Office.
- Windows PC Health Check, which came with KB5005463 on Windows 10.
- Teams Machine-Wide Installer, which puts the retired classic Teams back for everyone who signs in.

### Windows no longer quietly installs promoted apps (entry "Automatic app installs")

- The Apps tab lists it as a recommended entry while it is on. Ticking it and **Remove selected** sets three values to 0:
  - The values are `SilentInstalledAppsEnabled`, `OemPreInstalledAppsEnabled` and `PreInstalledAppsEnabled`.
  - They are in `HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager`.
- Without this, Windows can quietly bring back Candy Crush and other promoted apps. On the PC that was checked, all three were 1.
- Once it is off, the end of the tab lists it under **Turned off** with **Turn on**, which sets the values to 1 again.
- Microsoft's own policies only work on Enterprise and Education, not on Pro:
  - `RemoveDefaultMicrosoftStorePackages`
  - `DisableWindowsConsumerFeatures`

### Tips, suggestions and ads (entry "Suggestions")

- Also recommended, for the signed-in account only, and undone the same way with **Turn on**.
- It sets these values to 0, the ones Win11Debloat's `Disable_Windows_Suggestions.reg` sets under `HKCU`:
  - In `ContentDeliveryManager`: `SubscribedContent-310093Enabled` (the welcome after updates), `SubscribedContent-338388Enabled` and `SystemPaneSuggestionsEnabled` (suggestions in Start), `SubscribedContent-338389Enabled` and `SoftLandingEnabled` (tips), `SubscribedContent-338393Enabled`, `-353694Enabled`, `-353696Enabled` and `-353698Enabled` (suggestions in Settings).
  - In `Explorer\Advanced`: `Start_IrisRecommendations` (recommendations in Start) and `ShowSyncProviderNotifications` (OneDrive and other cloud ads in File Explorer).
  - `UserProfileEngagement\ScoobeSystemSettingEnabled` ("finish setting up your PC").
  - `Notifications\Settings\Windows.SystemToast.Suggested\Enabled` (notifications that suggest Microsoft's services).
- A missing value counts as on, as Windows treats it. Missing keys are created.
- Left out from the same file:
  - The HKLM policies `DisableCopilotPinScreen` and `HideRecommendedPersonalizedSites`, which need administrator rights.
  - `Mobility\OptedIn`, `Start_AccountNotifications`, `EnableAccountNotifications` and the backup reminder. They are about the phone, the account and backups rather than ads.

## Still open: needs administrator rights (one UAC prompt, like Clear DNS cache)

### Services

Most candidates are already set to Manual and only start when something needs them, so turning them off gains nothing.

| Service | On the PC that was checked | Suggestion |
|---|---|---|
| `DiagTrack` (telemetry) | Automatic, running | Disable, as WinUtil does. Windows Insider builds stop working. Nothing else changes. |
| `WSAIFabricSvc` (AI service, since 24H2) | Automatic, running | Set to Manual, as Win11Debloat does. It still starts when an AI feature needs it. |
| `MapsBroker` | Automatic, stopped | Set to Manual, as WinUtil does. It brings little. |
| `TrkWks` (Distributed Link Tracking) | Automatic, running | Optional. It does nothing useful on a home PC. |

Leave these services alone:

| Service | Why |
|---|---|
| `SysMain`, `WSearch` | Search and start-up times would suffer. |
| `Spooler` | Printing, including Microsoft Print to PDF. |
| `lfsvc` | Location: automatic time zone and local weather. It is already Manual. |
| The Xbox services | Saved games depend on them. They are already Manual. |
| `dmwappushservice` | Intune and other device management need it. |
| `PcaSvc`, `DPS` | Automatic. They warn about incompatible programs and run the troubleshooters, and cost almost nothing. |
| `WerSvc` | Already Manual. |

### Scheduled tasks

Sophia turns off these tasks, among others:

| Task | What it is |
|---|---|
| `Consolidator`, `UsbCeip` | Customer Experience Improvement Program |
| `Proxy`, `MareBackup` | Telemetry |
| `StartupAppTask` | Checks start-up apps |
| `Microsoft-Windows-DiskDiagnosticDataCollector` | Disk diagnostics |
| `MapsToastTask`, `MapsUpdateTask` | Offline maps |

Don't turn off **Microsoft Compatibility Appraiser**. Windows Update uses its data to decide whether a PC gets a feature update, so an update app shouldn't touch it.

### For every account, and after feature updates

- The app removes apps for the signed-in account only.
- Removing the provisioned package as well would keep an app away for new accounts and after feature updates.
- That takes administrator rights. The calls would be `Remove-AppxProvisionedPackage -Online` or `PackageManager.DeprovisionPackageForAllUsersAsync`.

### Optional Windows features (DISM)

| Feature | Windows | Note |
|---|---|---|
| Steps Recorder (`App.StepsRecorder`) | 10 and 11 | Deprecated by Microsoft. |
| Windows Media Player Legacy (`Media.WindowsMediaPlayer`) | 10 and 11 | |
| Internet Explorer 11 (`Browser.InternetExplorer`) | 10 only | |
| WordPad | 10 only | |
| XPS Viewer | 10 only | |
| PowerShell 2.0, SMB1 | 10 only | Removing them also makes the PC safer. |
| Recall | 11, Copilot+ PCs only | |

### More settings from Win11Debloat

- `PreventDeviceMetadataFromNetwork` (an HKLM policy) stops Windows from fetching makers' apps for connected devices.
- The HKCU part of `Disable_Windows_Suggestions` is built in, as the entry "Suggestions" (see above).

## Deliberately left out

| What | Why |
|---|---|
| PC makers' tools: MSI Center, Lenovo Vantage (`E046963F.LenovoCompanion`), MyASUS, Dell SupportAssist, HP Support Assistant, OMEN Gaming Hub | They deliver drivers, BIOS updates, and fan and battery settings. |
| The system package `MicrosoftWindows.Client.CoreAI` | Removing it, as WinUtil does, changes Windows itself. |
| Frameworks and extensions: `Microsoft.Advertising.Xaml`, VCLibs, the codec extensions (AV1, HEVC, HEIF and others) | Other apps need them. |
| Bought games, such as `Microsoft.MinecraftUWP` and the Minecraft Launcher (`Microsoft.4297127D64EC6`) | They were bought. |
| Dolby Access, Realtek Audio Console, NVIDIA Control Panel | They come with the drivers. |
| Edge Legacy (`Microsoft.MicrosoftEdge`) and the old Cortana (`Microsoft.Windows.Cortana`) | System packages that can't be removed. |
| Copilot from winget (`XP9CXNGPPJ97XX`) | `Microsoft.Copilot` is already on the list. |
| `Microsoft.StorePurchaseApp`, `Microsoft.BioEnrollment`, `Microsoft.Appconnector` | Windows needs them. |
