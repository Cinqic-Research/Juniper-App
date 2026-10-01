# Android release signing runbook

**Status: completed 2026-09-02.** The release keystore exists and all four
repository secrets are installed on `Cinqic/Juniper-App`. Nothing here needs to
be run again unless the keystore is rotated or the secrets are lost.

The keystore, its password, and a plain-language recovery note live in
`~/Documents/Juniper-Signing/` (directory `700`, files `600`). That folder is
the **only** copy of the password — GitHub secrets cannot be read back out.
Back it up off this machine.

| Property    | Value                                   |
| ----------- | --------------------------------------- |
| File        | `juniper-release.jks`                   |
| Format      | PKCS12                                  |
| Alias       | `juniper`                               |
| Key         | 4096-bit RSA, SHA384withRSA             |
| Subject     | `CN=Cinqic, OU=Juniper, O=Cinqic, C=US` |
| Valid until | 2054-01-18                              |

Verified end to end: the base64 secret round-trips to a byte-identical
keystore, and the decoded keystore opens with alias `juniper` under the stored
password — the same sequence `release.yml` performs.

The steps below are retained for rotation and disaster recovery.

## Why this is yours to do

The Android signing key permanently determines who can publish updates for
`com.cinqic.juniper`. If it is lost, you cannot ship an update to anyone who
installed a build signed with it — they must uninstall and reinstall. If it
leaks, someone else can sign builds that Android will accept as yours. Treat it
like a root credential.

## 1. Generate the release keystore

Pick a strong, unique password. You will be prompted for it twice, plus your
name/organization details.

```bash
keytool -genkeypair -v \
  -keystore juniper-release.jks \
  -alias juniper \
  -keyalg RSA -keysize 4096 -validity 10000 \
  -storetype JKS
```

Use the **same value** for the store password and the key password unless you
have a reason not to — `release.yml` supports them differing, but keeping them
equal is simpler and no less safe here.

## 2. Back it up before you do anything else

```bash
cp juniper-release.jks ~/secure-backup-location/
```

Store the file and its passwords in your password manager. A keystore with a
forgotten password is the same as a lost keystore. `*.jks` and `*.keystore` are
already gitignored — never commit it.

## 3. Encode it for GitHub

```bash
base64 -w0 juniper-release.jks > juniper-release.jks.b64
```

## 4. Install the four repository secrets

These are the names `release.yml` actually reads. (The earlier pre-audit
assumed `TAURI_ANDROID_*` names; those are not what the workflow uses.)

```bash
gh secret set ANDROID_KEYSTORE_BASE64 --repo Cinqic/Juniper-App < juniper-release.jks.b64
```

```bash
gh secret set ANDROID_KEY_ALIAS --repo Cinqic/Juniper-App --body "juniper"
```

```bash
gh secret set ANDROID_KEYSTORE_PASSWORD --repo Cinqic/Juniper-App
```

```bash
gh secret set ANDROID_KEY_PASSWORD --repo Cinqic/Juniper-App
```

The last two prompt for the value so it stays out of your shell history.

## 5. Remove the encoded copy

```bash
shred -u juniper-release.jks.b64
```

Keep `juniper-release.jks` itself, backed up per step 2.

## 6. Confirm

```bash
gh secret list --repo Cinqic/Juniper-App
```

You should see exactly these four:

```
ANDROID_KEYSTORE_BASE64
ANDROID_KEY_ALIAS
ANDROID_KEYSTORE_PASSWORD
ANDROID_KEY_PASSWORD
```

`release.yml` asserts all four are non-empty and refuses to publish an unsigned
APK, so a missing secret fails the build rather than shipping something
unsigned.

## What happens next

`publish` needs `verify`, which needs `linux`, `windows`, **and** `android`. So
until these secrets exist, a `v*` tag push fails at the Android job and
publishes nothing — safe, but it leaves a permanent tag and a failed run.

Once the secrets are in place, tagging is the trigger:

```bash
git tag v0.2.0-rc.1 && git push origin v0.2.0-rc.1
```

Be deliberate: `on: push: tags: ['v*']` sets `publish=true` automatically, so
that tag push **is** the public release. `check-version.mjs` requires the tag to
be exactly `v0.2.0-rc.1` while `package.json` reads `0.2.0-rc.1`; promoting to
`v0.2.0` means bumping the version across `package.json`, `Cargo.toml`,
`tauri.conf.json`, and `manifests/release-candidate.yaml` first, along with the
`expectedAndroidVersionCodes` and `expectedMsiVersions` maps in
`scripts/check-version.mjs`.

Only after the artifacts exist and their `SHA256SUMS` verify should the website
download links be pointed at them.
