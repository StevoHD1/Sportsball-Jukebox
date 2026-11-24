# Installing Sportsball Jukebox

**Requirements:** macOS 14.2 (Sonoma) or later, Apple Silicon or Intel.

1. Download `Sportsball-Jukebox-<version>.dmg` (from the release, or from
   a CI run's **Sportsball-Jukebox-macOS** artifact, which is a zip
   containing the disk image).
2. Open the disk image and drag **Sportsball Jukebox** onto
   **Applications**.
3. Open it from Applications. The first time, macOS says it can't verify
   the app, because it isn't notarized yet (that needs an Apple Developer
   account). Click **Done**, open **System Settings → Privacy &
   Security**, scroll down and click **Open Anyway** next to Sportsball
   Jukebox, and confirm. If that button doesn't appear, run this in
   Terminal:

   ```sh
   xattr -dr com.apple.quarantine "/Applications/Sportsball Jukebox.app"
   ```

4. If macOS asks whether Sportsball Jukebox may find devices on your
   local network, click **Allow**. The phone remote and OSC need it. If
   you clicked Don't Allow, change it in **System Settings → Privacy &
   Security → Local Network**.

Steps 3 and 4 are needed once per new version. Shows, settings and the
game log are kept when you update.

See `docs/testing-on-mac.md` for what to try.

## Making a release

Push a version tag; CI builds, tests and publishes the disk image as a
GitHub Release (a tag with a suffix, like `v0.2.0-beta.1`, is marked as a
pre-release):

```sh
git tag v0.1.0
git push origin v0.1.0
```

Or on GitHub: **Releases → Draft a new release → Choose a tag**, type
`v0.1.0`, **Create new tag on publish**, and publish. CI then attaches
the disk image to that release a few minutes later.
