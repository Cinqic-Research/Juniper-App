# Juniper 0.4.0-Alpha publication record

| Item | Value |
|---|---|
| Display name / machine version / tag | Juniper 0.4.0-Alpha / `0.4.0-alpha.1` / `v0.4.0-alpha.1` (Android versionCode 4001, MSI 0.4.0.1) |
| Release commit | `96ee95d384383f64bd958f282732fca780fbaddb` (merge of PR #52 into `main`, 2026-10-17) |
| Gate G1 | validation run [39031231599](https://github.com/Cinqic-Research/Juniper-App/actions/runs/39031231599) on the release commit: every job success (Windows MSI install/render/uninstall smoke; sandbox evidence 6 passed / 0 failed; Linux bundle build and both launch smokes; Android clean debug compile; security audit); the Android native inference manual job skipped by design |
| Release workflow | run [39032524940](https://github.com/Cinqic-Research/Juniper-App/actions/runs/39032524940): immutable release contract, Windows MSI install smoke, Linux build/install smoke, signed Android APK smoke, Ubuntu 24.04 launch smoke, downloaded-artifact verification and provenance, atomic prerelease publication: all success |
| Release page | <https://github.com/Cinqic-Research/Juniper-App/releases/tag/v0.4.0-alpha.1> (prerelease; not marked latest) |
| Published 2026-10-17 (US Eastern) | about 08:55 AM EDT |

## Artifacts (SHA-256 from the release `SHA256SUMS.txt`, re-verified after a fresh download on FLOWBOX)

```text
ab783fb8dcd49398d2cd31859857dd70ebb8f3066d5c0b2d33d0476d35e0b792  Juniper-0.4.0-alpha.1-android-native-symbols.zip
8b319798a1368392dbc55a0b2be575c63357493d3521b9c781574e4456346df4  Juniper-0.4.0-alpha.1-android-universal.apk
71528f6da6ac52cdb09cdbe8f5308e4894ec8c8d5c61fb5e0539bcc64e6a8d5a  Juniper-0.4.0-alpha.1-linux-x86_64.AppImage
9fb1efa68906b1e54af5c7c183ebea979860d1d75b97f498ffc64ee5b6fef984  Juniper-0.4.0-alpha.1-linux-x86_64.deb
9477aecbac413310c7e52c6a7e456e0b943ba9f811a4eaccd517a1ca5847391d  Juniper-0.4.0-alpha.1-windows-x86_64.msi
```

Also published: `SIGNING-android.txt`, `SIGNING-windows.txt` (MSI unsigned), `PACKAGE-*.txt`, `RELEASE-PROVENANCE.json`, `LICENSE`, `THIRD_PARTY_NOTICES.md`.

## Read-back

- All five artifacts and `SHA256SUMS.txt` downloaded anonymously with `gh release download`; `sha256sum -c`: OK for every file.
- The AppImage launched on FLOWBOX (X11) for 10 s with an isolated `HOME`/XDG profile: startup log reports `Juniper 0.4.0-alpha.1 starting on linux/x86_64`, local runtime found; the owner's database was not opened.
- cinqic.com Juniper and Apps pages updated (PR #26) after `scripts/verify_release_links.py` passed against the published assets.

## Status as released

Linux .deb/.AppImage supported; Windows MSI supported, unsigned (SmartScreen warning), Agent on Windows not verified; Android APK Beta pending a physical ARM64 inference run; backup restore not verified on a clean machine; five Llama 3.x / Gemma 3 catalog entries held (third-party mirrors; owner decision pending). Gates and evidence: `juniper-0.4.0-alpha-readiness-review.md`; non-implementer qualification records: `../qualification/0.4.0-alpha-qualification-2026-10-17/`.
