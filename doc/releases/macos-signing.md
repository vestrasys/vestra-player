# macOS signing and notarization: setup

`release.yml`'s `sign-macos` job signs the macOS packages with a **Developer ID Application** certificate, notarizes them with an **App Store Connect API key** and staples the tickets. It needs five secrets and one repository variable (the Team ID, which is not secret). This page says how to get each one and where to put it. Do it once on a Mac signed in to the Apple Developer account (the Account Holder, for step 1).

| Secret or variable | What it is |
| --- | --- |
| `MACOS_CERT_P12_BASE64` | the Developer ID Application certificate with its private key, as a `.p12`, base64 |
| `MACOS_CERT_PASSWORD` | the password you set when exporting the `.p12` |
| `APPLE_API_KEY_P8_BASE64` | the App Store Connect API key file `AuthKey_<KEY ID>.p8`, base64 (its plain text also works) |
| `APPLE_API_KEY_ID` | that key's Key ID (10 characters) |
| `APPLE_API_ISSUER_ID` | the Issuer ID shown above the Team Keys list (a UUID) |
| `APPLE_TEAM_ID` (**variable**, not a secret) | the team's 10-character Team ID |

No keychain password is needed: the job makes a temporary keychain with a random password and deletes it at the end.

## 1. Developer ID Application certificate → `MACOS_CERT_P12_BASE64`, `MACOS_CERT_PASSWORD`

1. If you do not have one yet: Xcode → Settings → Accounts → select the team → Manage Certificates… → `+` → **Developer ID Application**. Only the Account Holder can create it. (Or use developer.apple.com → Certificates → `+` → Developer ID Application, with a certificate signing request from Keychain Access → Certificate Assistant.)
2. Keychain Access → login keychain → **My Certificates** → find `Developer ID Application: <name> (<TEAM ID>)`. It must have a disclosure triangle with a private key under it; without the key the export is useless.
3. Right-click it → Export… → format **Personal Information Exchange (.p12)** → save as `DeveloperID.p12` → set a strong password. That password is `MACOS_CERT_PASSWORD`.
4. Base64 it straight into the secret (step 4 below), or to the clipboard: `base64 -i DeveloperID.p12 | pbcopy`.

## 2. App Store Connect API key → `APPLE_API_KEY_P8_BASE64`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID`

1. appstoreconnect.apple.com → Users and Access → **Integrations** → App Store Connect API → **Team Keys**. The first time, the Account Holder has to click "Request Access" and accept.
2. `+` (Generate API Key): name `vestra-notary`, Access **Developer** → Generate.
3. **Download** the key: `AuthKey_<KEY ID>.p8`. Apple lets you download it only once; keep it safe.
4. The **Key ID** is in the key's row (`APPLE_API_KEY_ID`). The **Issuer ID** is shown above the list (`APPLE_API_ISSUER_ID`).

## 3. Team ID → variable `APPLE_TEAM_ID`

developer.apple.com/account → Membership details → **Team ID**. It is also the 10 characters in brackets in the certificate's name. The job checks that the `.p12` holds a Developer ID Application identity for this team, and that every signed file carries it.

## 4. Add the secrets and the variable to GitHub

Put the five secrets on the `release` environment (Settings → Environments → `release` → Environment secrets → Add secret), which is what the job runs in. The Team ID is a repository variable (Settings → Secrets and variables → Actions → Variables → New repository variable). With `gh`, from the directory with the two files:

```
R=vestrasys/vestra-player
base64 -i DeveloperID.p12          | gh secret set MACOS_CERT_P12_BASE64   --env release -R $R
gh secret set MACOS_CERT_PASSWORD  --env release -R $R      # prompts; paste the .p12 password
base64 -i AuthKey_XXXXXXXXXX.p8    | gh secret set APPLE_API_KEY_P8_BASE64 --env release -R $R
gh secret set APPLE_API_KEY_ID     --env release -R $R -b XXXXXXXXXX
gh secret set APPLE_API_ISSUER_ID  --env release -R $R -b xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
gh variable set APPLE_TEAM_ID      -R $R -b XXXXXXXXXX
gh secret list --env release -R $R && gh variable list -R $R
```

If the repository's plan does not offer environment secrets (a private repository on GitHub Free), leave out `--env release` to make them repository secrets; the job reads those too. In the `release` environment's **Deployment branches and tags**, choose "Selected branches and tags" (not "All branches") and add the branch `main` and the tag pattern `v*`: only release tags and dry runs dispatched from `main` can then read the secrets. Delete the local `.p12` and keep the `.p8` somewhere safe (a password manager), not in the repository.

## 5. Test, then release

- **Dry run with signing** (macOS only, to save Windows minutes):
  ```
  gh workflow run release.yml -R vestrasys/vestra-player --ref main \
    -f dry_run=true -f platforms=macos -f sign_macos=true
  ```
  `sign-macos` and then `smoke test (signed macOS)` (the packaged smoke test on the signed DMG's app) must end green: `sign-macos` prints each signed file, the notarization log of the `.app` and of the DMG, and the `spctl` lines `source=Notarized Developer ID`. The signed files are the run's artifact `vestra-signed-macos` (`gh run download <run id> -n vestra-signed-macos`, kept 1 day). Download the DMG through a browser on a Mac (so it gets the quarantine flag), open it and start the app: there must be no Gatekeeper warning.
- **Release**: tag as usual (README "Release steps"). A tag release always signs; without the secrets `sign-macos` fails and no draft release is made. A dry run without the secrets and with `sign_macos` warns and goes on with the unsigned packages.

## What the job does

From the packaged `.app.zip` (`vestra-unsigned-macos`): every nested Mach-O (Frameworks/*.dylib, ffmpeg, ffprobe and any other found) signed inside out with the Hardened Runtime and a secure timestamp, then `MacOS/node` with `src/packaging/macos/node.entitlements` (V8's JIT), then the host and the bundle with `host.entitlements` (empty); no `--deep` signing. Each file is checked for the team, the runtime flag and the timestamp; a nested `.framework`, `.app`, `.xpc` or `.bundle` under `Contents` fails the job (none is expected). Apple's Developer ID G2 intermediate is fetched from apple.com and pinned by SHA-256. Timestamp-server errors and `stapler staple` are retried (15 s apart); each notarization waits up to 30 minutes, the job up to 100. The `.app` is notarized (`xcrun notarytool submit --wait`) and stapled, zipped as the release's `.app.zip`, and put into a new DMG (same install window as `bundle-macos.mjs`: `dmgbuild` with `src/packaging/macos/dmg/settings.py`, or plain `hdiutil` with the repository variable `VESTRA_DMG_PLAIN=1`), which is signed, notarized and stapled. Checks: `codesign --verify --deep --strict`, `spctl -a -t exec` on the app and `spctl -a -t open --context context:primary-signature` on the DMG (both must say `source=Notarized Developer ID`; `-t install` is for `.pkg` installers), `stapler validate` on both, on the files as uploaded. The macOS `host.json` entry is rewritten with the signed host binary's SHA-256, so a later release can reuse it. A follow-on job without secrets (`smoke-macos-signed`) mounts the signed DMG and runs `smoke.mjs` on its app; the draft release waits for it.

Native Node addons (`.node`) are not supported on signed builds: node is signed without `disable-library-validation`, so it loads only code signed by our team. Store plugins cannot ship them anyway (the plugin repository's `check-package.mjs` rejects native code).

The certificate is valid for 5 years; notarized and timestamped releases keep working after it expires. Renew it, export a new `.p12` and replace the two certificate secrets. A revoked API key only needs a new key and the three key secrets.
