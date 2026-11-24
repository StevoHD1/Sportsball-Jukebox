# Signing and notarizing (when there is an Apple Developer account)

Today CI produces an **ad-hoc signed** app. It runs on Apple Silicon and
Intel, but Gatekeeper blocks the first launch until it's approved in
System Settings (see `testing-on-mac.md`). That's fine for testing. For
shipping to other people the app should be signed with a Developer ID
and notarized by Apple, so it opens like any other app.

## What you need

1. An Apple Developer Program membership ($99/year).
2. A **Developer ID Application** certificate:
   Xcode → Settings → Accounts → Manage Certificates → + → Developer ID
   Application. Export it from Keychain Access as a `.p12` with a
   password.
3. An **app-specific password** for notarization
   (appleid.apple.com → Sign-In and Security → App-Specific Passwords),
   and your **Team ID** (developer.apple.com → Membership).

## CI secrets to add

In GitHub → Settings → Secrets and variables → Actions:

| Secret | Value |
|---|---|
| `MACOS_CERT_P12_BASE64` | `base64 -i cert.p12` |
| `MACOS_CERT_PASSWORD` | the .p12 password |
| `APPLE_ID` | your Apple ID email |
| `APPLE_APP_PASSWORD` | the app-specific password |
| `APPLE_TEAM_ID` | your Team ID |

## Workflow changes

Once those exist, the macOS job gets these steps (to be added then;
they can't be tested without the account):

1. Import the certificate into a temporary keychain
   (`security create-keychain`, `security import`,
   `security set-key-partition-list`).
2. Build with `CODE_SIGN_IDENTITY="Developer ID Application"`,
   `DEVELOPMENT_TEAM=$APPLE_TEAM_ID`, `ENABLE_HARDENED_RUNTIME=YES` and
   `OTHER_CODE_SIGN_FLAGS=--timestamp`.
3. Zip the app, then submit it with
   `xcrun notarytool submit app.zip --apple-id … --team-id … --password … --wait`.
4. Staple the ticket: `xcrun stapler staple "Sportsball Jukebox.app"`,
   then zip it again for release.

**Entitlements:** with the hardened runtime, the app needs no special
entitlements for audio output, MIDI or network listening (incoming OSC).
Sandboxing (needed only for the Mac App Store) would additionally need
`com.apple.security.network.server` and user-selected file access.
