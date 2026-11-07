# Releasing Soulcaster

The version is set in one place: `[workspace.package]` in `Cargo.toml`. A release is a `v*` tag that matches it, such as `v0.1.0`. Pushing the tag runs `.github/workflows/release.yml`.

## What a release does

1. Builds, tests and packages the app, ad-hoc signed.
2. If all the Apple secrets are set, signs it with Developer ID and the hardened runtime, notarizes it and staples the ticket. It stops if some of the secrets are set and others aren't.
3. Publishes a GitHub release with `Soulcaster-macos-arm64.zip` and `Soulcaster-macos-arm64.zip.sha256`.

| Apple secrets | What gets published | Can the installer get it? |
| --- | --- | --- |
| All set | A normal release, marked latest | Yes |
| None set | A prerelease titled "(not notarized)" | No: `releases/latest` skips prereleases, and the installer would refuse it anyway |

## Until a notarized release exists

`install.sh` stops with a message pointing to the source build:

```sh
git clone https://github.com/nullaroni/soulcaster.git
cd soulcaster
packaging/build-app.sh
open target/package/Soulcaster.app
```

The build needs Rust and Command Line Tools. It installs nothing, and it leaves an ad-hoc signed app in `target/package/`.

## Enrolling and turning on notarization

Only the owner can do these steps.

1. **Join the Apple Developer Program** (99 USD a year) at <https://developer.apple.com/programs/enroll/>. An individual enrollment shows your legal name in every copy of the app. An organization needs a legal entity and a D-U-N-S Number.
2. **Create a Developer ID Application certificate.** The Account Holder does this at <https://developer.apple.com/account/resources/certificates/list>, or in Xcode under Settings > Accounts > Manage Certificates. Choose Developer ID Application, not Developer ID Installer.
3. **Export it as a `.p12`** from Keychain Access, with a strong password. Keep a backup somewhere outside this repository.
4. **Create a notary key.** In App Store Connect, go to Users and Access > Integrations > App Store Connect API > Team Keys and generate a key. Download the `.p8` file (Apple offers it only once), and note its Key ID and the Issuer ID. Apple's roles page didn't settle the minimum access level during research, so confirm it on the first submission.
5. **Add these repository secrets** under Settings > Secrets and variables > Actions:

   | Secret | Value |
   | --- | --- |
   | `APPLE_CERTIFICATE_P12_BASE64` | `base64 -i certificate.p12` |
   | `APPLE_CERTIFICATE_PASSWORD` | The password from step 3 |
   | `APPLE_NOTARY_KEY_P8_BASE64` | `base64 -i AuthKey_XXXXXXXXXX.p8` |
   | `APPLE_NOTARY_KEY_ID` | The Key ID from step 4 |
   | `APPLE_NOTARY_ISSUER_ID` | The Issuer ID from step 4 |

6. **Set the Team ID in `install.sh`.** In `soulcaster_main`, set `local team_id=""` to your 10-character Apple Team ID, which is on the Membership page of your account. The release stops if the signing certificate's team doesn't match this value.
7. **Review `CFBundleIdentifier` in `packaging/Info.plist`** before the first public release. It is `com.nullworks.macflayer`, chosen by the owner, and changing it later makes macOS treat the app as a different one.
8. **Tag it:**

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

To redo a release for a tag that has already been published, for example after adding the secrets, delete the old release with `gh release delete v0.1.0 --yes`, keeping the tag, and re-run the workflow.

## Checks

```sh
packaging/build-app.sh                  # build, ad-hoc sign and check the bundle
/bin/bash packaging/test-install.sh     # test the installer with stubs in a sandbox
```

`packaging/icon/icon-1024.png` is rendered from `icon.svg`. After changing the SVG, run:

```sh
sips -s format png packaging/icon/icon.svg --out packaging/icon/icon-1024.png
```

## Not proven yet

- The tag path of the release workflow has not run. It was checked by reading it and by running its build and archive scripts locally.
- The signing and notarization path can't run until the owner enrolls. Entitlements are none for now, and a real notarized build could show that the app needs some.
- No run has happened on a bare Mac. Real-Mac verification is the last ticket on tracker #20, and it needs the owner's go-ahead.
