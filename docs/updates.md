# Updates: "Update now / Later"

Every time Zipkoo starts, it checks for a newer version. If one exists, it asks:

> **Zipkoo 0.2.0 is ready** — what's new — **Later** · **Update now**

- **Update now:** downloads the update, installs it, and restarts Zipkoo.
- **Later:** closes the popup. Zipkoo asks again the next time it starts.
- Settings → Updates has a manual "Check for updates" button and an option to switch off the check at launch.
- The Mac App Store build never shows this, because the App Store handles its updates.

## One-time setup (you, on your Mac)

Zipkoo only installs updates that are signed with your key, so nobody else can push a fake update.

1. Make the key pair (no password, so only one secret is needed):

```bash
npx @tauri-apps/cli signer generate -w ~/.tauri/zipkoo.key -p "" --ci
```

2. **Back up `~/.tauri/zipkoo.key`** (a password manager is fine). If you lose it, the copies people already installed can never update again.
3. On GitHub, go to repo → Settings → Secrets and variables → Actions → New repository secret, and add:
   - `TAURI_SIGNING_PRIVATE_KEY`: the full contents of `~/.tauri/zipkoo.key` (the only secret needed)
4. Send Claude the contents of `~/.tauri/zipkoo.key.pub`. It is public and safe to share. It goes into `app/src-tauri/tauri.conf.json` (`plugins.updater.pubkey`), and `bundle.createUpdaterArtifacts` gets turned on.

Only builds made **after** step 4 (done: the public key is in tauri.conf.json) can update themselves. Install that build once by hand; every later version then arrives through the popup.

## Releasing an update

1. Raise the version in `app/src-tauri/tauri.conf.json` and `app/package.json` (e.g. `0.2.0`).
2. Tag and push:

```bash
git tag v0.2.0
```

```bash
git push origin v0.2.0
```

3. The Release workflow builds the Windows and Mac installers plus `latest.json`, all signed, and makes a **draft** release.
4. Open the draft on GitHub, write what's new (this text appears in the popup), and press **Publish**. From then on every Zipkoo that starts sees the popup.
