# Zipkoo on the Mac App Store

Zipkoo has two builds:

| | Direct download (`direct`, default) | Mac App Store (`appstore`) |
|---|---|---|
| Sandbox | No | Yes (required by Apple) |
| Updates | Built-in signed updater | The App Store |
| Finder right-click Quick Actions | Yes (writes to `~/Library/Services`) | No. "Open With" and the file types Zipkoo declares do the job instead |
| Self-extracting `.exe` | Yes (Windows only) | No |
| Virus scan after extracting | Uses Defender or ClamAV if installed | No (a sandboxed app can't run other programs) |
| Folder access | Any folder | Only folders the person chose. Zipkoo asks once and remembers (security-scoped bookmarks) |

Everything else is the same, including opening, extracting, creating, editing, repairing, Secure Share, saved passwords (Keychain) and the 20 GB+ support.

## How the sandbox is handled

- **Extract here / Zip here / editing / repair data / recovering files** all write next to the archive. Before writing, Zipkoo checks it is allowed to (`folder_access`). If it isn't, it explains why and opens a folder picker on that folder. The person's choice is saved as a security-scoped bookmark (`folder_grant`) and restored at every launch (`access::restore`). It never fails halfway through.
- **Watch folders, search folders and share download folders** are saved as bookmarks the moment they are picked, so backups keep running after a restart.
- **Recent files** that can no longer be read after a restart show the same "choose its folder" prompt instead of an error.
- **Move to Trash** uses `NSFileManager`, not Finder scripting, so it needs no Apple Events permission.
- **Temporary files** (previews, "open with", drag-out) live in the app's own cache folder inside the sandbox container.

Files: `app/src-tauri/src/access.rs`, `app/src-tauri/macos/Zipkoo.appstore.entitlements`, `app/src-tauri/macos/PrivacyInfo.xcprivacy`, `app/src-tauri/tauri.appstore.conf.json`, `scripts/macos-appstore.sh`.

Entitlements, kept to the minimum:
`app-sandbox`, `files.user-selected.read-write`, `files.bookmarks.app-scope`, `network.client` (Secure Share only).

## One-time setup with Apple (you do this; it costs money and needs your identity)

1. Join the **Apple Developer Program** (USD 99/year) at developer.apple.com.
2. **Certificates** (Xcode → Settings → Accounts → Manage Certificates, or the website):
   - *Apple Distribution*: signs the app.
   - *Mac Installer Distribution* (shows as "3rd Party Mac Developer Installer"): signs the `.pkg`.
3. **Identifiers → App ID**: explicit bundle ID `app.zipkoo.desktop`. No extra capabilities are needed.
4. **Profiles → Mac App Store Connect** provisioning profile for that App ID. Download it.
5. **App Store Connect → Apps → +**: platform macOS, bundle ID `app.zipkoo.desktop`, SKU e.g. `zipkoo-mac`.

## Build and upload (on a Mac with Xcode)

```bash
APPLE_TEAM_ID=ABCDE12345 \
APP_IDENTITY="Apple Distribution: Your Name (ABCDE12345)" \
INSTALLER_IDENTITY="3rd Party Mac Developer Installer: Your Name (ABCDE12345)" \
PROVISION_PROFILE=~/Downloads/Zipkoo_Mac_App_Store.provisionprofile \
./scripts/macos-appstore.sh
```

The script:
1. Builds a universal (Apple silicon + Intel) app with `--features appstore --no-default-features`.
2. Signs it with the sandbox entitlements.
3. Checks the signature.
4. Makes `Zipkoo-<version>.pkg`.

It does **not** upload. Upload the `.pkg` with the **Transporter** app.

Before sending it for review, test the signed build on a Mac. Install it from TestFlight, or run it from the `.app`:
- Open an archive from Finder, then choose Extract. Zipkoo should ask for that folder once, then extract.
- Quit, reopen, and extract again in the same folder. It should not ask again.
- Add a watch folder, restart, and press Back up now. It should work.
- Run Console.app and filter for `Sandbox`: there should be no `deny` lines for Zipkoo during normal use.

## App Review guideline pass

| Guideline | Status |
|---|---|
| 2.4.5(i) Sandboxed, only needed entitlements | Done. 4 entitlements, each explained above |
| 2.4.5(ii) Packaged and submitted with Apple's tools | `productbuild` + Transporter |
| 2.4.5(iii) Single app bundle, nothing installed elsewhere | Done. The App Store build writes no Quick Actions, login items or helpers |
| 2.4.5(iv) No auto-launch or code at login without consent | Done. Watch-folder backups only run while Zipkoo is open |
| 2.4.5(vii) No own update mechanism | Done. The updater plugin isn't compiled into the App Store build; Settings says "Updates come from the App Store" |
| 2.4.5(viii) No admin rights, no changing other apps | Done |
| 2.5.1 Public APIs only | Done (Tauri/WebKit, Foundation `NSURL` bookmarks, Security keychain) |
| 2.5.2 No downloading or running code | Done. No SFX stub and no external scanner in this build |
| 2.1 App completeness | Every visible feature works offline. Secure Share needs a share server: without one, Zipkoo says so and links to the setting |
| 2.3 Accurate metadata | Don't mention Windows-only features (SFX, right-click menu, virus scan) in the Mac App Store description or screenshots |
| 4.0 Design / Mac conventions | Default macOS app menu (Edit, Window, Quit), Dark Mode, resizable window (min 860×560), keyboard navigation |
| 5.1.1 Privacy policy | **Needed**: a public URL (see below) |
| 5.1.2 Data use | Collects nothing. Privacy manifest declares no tracking, no collected data, and why file dates, disk space and settings APIs are used |
| Export compliance | Zipkoo uses AES (ZIP/7z passwords, Secure Share). In App Store Connect, answer **Yes** to "uses encryption". It is standard encryption for protecting the user's own data, which normally qualifies for the mass-market exemption. This is a legal answer: confirm it yourself (or with an advisor) before submitting |

## App Store Connect metadata checklist

- [ ] Name: **Zipkoo**. Subtitle (30 chars), e.g. "Zip it. Ship it."
- [ ] Category: **Utilities** (already set in the bundle).
- [ ] Description and keywords. Mac features only.
- [ ] Screenshots: at least one at 1280×800, 1440×900, 2560×1600 or 2880×1800. Light and dark look good.
- [ ] **Privacy Policy URL** (required). A short page is enough: "Zipkoo does not collect any data. Files you share go only to the share server you set."
- [ ] Support URL (a page or the GitHub issues link).
- [ ] App Privacy questionnaire: **Data Not Collected**.
- [ ] Age rating questionnaire: all "None", which gives 4+.
- [ ] Price and availability.
- [ ] Notes for the reviewer, for example: "No account needed. To try it, open any .zip/.7z/.rar from Finder, or drag files onto the window to make one. macOS asks once for folder access when extracting next to an archive; this is the sandbox working as intended. Secure Share needs a share server, which is optional."
- [ ] Version and build number match `app/src-tauri/tauri.conf.json`.

## What was tested here and what wasn't

Tested on Linux (CI also compiles the App Store flavour on macOS):
- The App Store flavour builds.
- 19 end-to-end checks on the real app:
  - no updater, SFX, external scanner or Quick Actions writer;
  - Settings shows no update button, integration section or scan toggle;
  - the folder-access prompt appears for a folder Zipkoo can't write to, and Cancel leaves nothing behind.
- 101 end-to-end checks on the normal build.

Not tested (needs a Mac, your Apple certificates and your account):
- The real sandbox at runtime.
- Bookmarks surviving a restart.
- Code signing, `productbuild`, Transporter upload, TestFlight, and App Review itself.
