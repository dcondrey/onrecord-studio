# Direct release runbook (Developer ID, notarized, self-updating)

App Store releases are covered by `release.md`. The App Store build never self-updates: the updater
plugin is linked only with the `direct-license` feature and its capability
(`src-tauri/capabilities/direct.json`) is listed only by `tauri.direct.conf.json`. An update replaces
the binary and nothing else; it never grants or extends a license.

## Environment

| name | purpose |
|---|---|
| `ONRECORD_SUPABASE_ANON_KEY` | public anon key, read by `prepare-direct.mjs` |
| `APPLE_DIRECT_PROVISIONING_PROFILE_PATH` | Developer ID provisioning profile |
| `APPLE_SIGNING_IDENTITY` | Developer ID Application identity (also signs the DMG) |
| `APPLE_NOTARY_KEYCHAIN_PROFILE` | `notarytool` keychain profile |
| `RELEASE_BASE_URL` | https base where the owner hosts artifacts; the app URL is `$RELEASE_BASE_URL/<version>/<file>` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | optional; otherwise read from `updater.key.password` |
| `ONRECORD_UPDATER_KEY_PATH`, `ONRECORD_UPDATER_KEY_PASSWORD_PATH` | optional overrides of the key locations |
| `RELEASE_NOTES_PATH` | optional; default `RELEASE_NOTES.md` at the app root |
| `ONRECORD_APP_BUNDLE_PATH` | optional; a non-default `.app` location |

`.env.direct.local` (git-ignored) may hold them; the shell wins.

## Release

From `onrecord-studio/`, with the version already agreed in `package.json`, `src-tauri/Cargo.toml`
and `src-tauri/tauri.conf.json` (`prepare-direct.mjs --check` fails if they disagree):

```sh
$EDITOR RELEASE_NOTES.md                 # plain text shown in the update prompt (first 400 characters)
node scripts/build-direct.mjs --check    # validates everything possible without credentials
node scripts/build-direct.mjs --dry-run  # prints the plan
node scripts/build-direct.mjs            # prepare, tauri build, notarize app, package
```

`package-direct.mjs` (last step) verifies the stapled `.app` (`codesign --verify --deep --strict`,
`stapler validate`, `spctl --assess`), then writes to `dist-tauri/release/<version>/`:

- `OnRecord-Studio_<version>_universal.dmg`: signed, notarized, stapled (hdiutil; no extra tool).
- `OnRecord-Studio_<version>_universal.app.tar.gz` and `.sig`: built from the stapled app, signed with
  the updater key, verified against the public key in `tauri.direct.conf.json`, then extracted and
  re-verified with codesign and stapler.
- `latest.json`.

Nothing is uploaded.

### Xcode 27.0 linker

Xcode 27.0 (27A266a) `ld`, at any deployment target of 12.0 or later (Tauri sets 13.0 from
`minimumSystemVersion`), does not pad after an odd-length indirect symbol table, so about half of
the linked images get a 4-byte-aligned string pool. macOS 27 dyld refuses such an image on `dlopen`
(the main executable still launches), so the build stops with `can't find crate for <some>_derive`
after `dlopen ... (mis-aligned LINKEDIT string pool)` on a host proc-macro. Debug builds are
unaffected (no deployment target). Linking without chained fixups gave all 45 proc-macro dylibs an
aligned pool (measured, not explained: the shipped executable itself stays unpadded and runs). Until
a fixed Xcode ships, set for the release build only, in `.env.direct.local`:

```sh
CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=/Volumes/A/onrecord-studio/onrecord-studio/scripts/xcode27-linker/cc
CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER=/Volumes/A/onrecord-studio/onrecord-studio/scripts/xcode27-linker/cc
CARGO_TARGET_DIR=/Volumes/C/rust-target-onrecord-release
```

The `[target]` linker applies to host proc-macros too; target `rustflags` would not. The separate
target directory keeps the changed linker from relinking everyone's shared dev artifacts.

## Feed format

The app requests `https://updates.writerslogic.com/onrecord/updates/{{target}}/{{arch}}/{{current_version}}`
(`target` is `darwin`, `arch` is `aarch64` or `x86_64`). A static host must answer every such path
with the same `latest.json` (a rewrite that ignores the trailing version segment); the plugin does
the version comparison itself.

```json
{
  "version": "1.0.1",
  "notes": "Text from RELEASE_NOTES.md",
  "pub_date": "2026-10-01T12:00:00.000Z",
  "platforms": {
    "darwin-aarch64": { "signature": "<contents of the .sig file>", "url": "https://.../1.0.1/OnRecord-Studio_1.0.1_universal.app.tar.gz" },
    "darwin-x86_64": { "signature": "<same>", "url": "<same>" }
  }
}
```

The archive is universal, so both entries carry the same signature and URL. Tauri 2 has no updater
`dialog` option; the app shows its own prompt (`src/components/updates/`).

## Owner-only steps

1. Notary credentials: `xcrun notarytool store-credentials <profile>` (needs an app-specific
   password); export `APPLE_NOTARY_KEYCHAIN_PROFILE`.
2. Host the DMG, `.app.tar.gz`, and `latest.json`. The Cloudflare worker `updates-writerslogic` (deployed) serves R2 bucket `writerslogic-releases` at `updates.writerslogic.com`: set `RELEASE_BASE_URL=https://updates.writerslogic.com/onrecord`, upload artifacts to `onrecord/<version>/` and the feed to key `onrecord/latest.json` (`wrangler r2 object put writerslogic-releases/onrecord/latest.json --file latest.json --content-type application/json --remote`). Until it exists the feed answers 204 (no update). Publish `latest.json` last: it is what makes clients update.
3. Set `DOWNLOAD_URL` in the website config to the hosted DMG.
4. Authorize the commit, tag, and release.

## Test an update locally

1. Build and package a higher version (for example 1.0.1) as above, with `RELEASE_BASE_URL` set to
   an **https** local address, for example `https://localhost:8443/releases` served through a local
   TLS proxy (e.g. `mkcert` plus a static server, or `npx local-ssl-proxy`). `package-direct.mjs`
   asserts `RELEASE_BASE_URL` matches `^https://` unconditionally, including in `--check`/`--dry-run`,
   so a plain `http://` value fails packaging before anything is written.
2. Serve `dist-tauri/release/1.0.1` behind that same https address so that `latest.json` is at the
   feed path and the archive at its URL (rewrite the three trailing path segments to `latest.json`).
3. Build a lower version (1.0.0) with a temporary endpoint override, for example
   `--config` with `{"plugins":{"updater":{"endpoints":["https://localhost:8443/latest.json"]}}}`
   (plain http is refused by the plugin in release builds unless `dangerousInsecureTransportProtocol`
   is set in that same temporary config, which still would not satisfy step 1's packaging-time check).
   Do not commit either override.
4. Launch the 1.0.0 app: the prompt shows 1.0.1. Install and restart, then confirm the version and
   that the account and license state are unchanged.

## Rollback

Republish the previous `latest.json` (same file, previous signature and URL). Clients already on the
bad version only move to a higher `version`, so to roll clients back ship a new, higher version that
contains the old code. Remove the bad DMG from the download page.

## Key custody and rotation

The updater key pair lives in `~/.config/onrecord-studio/signing/`: `updater.key` (private) and
`updater.key.password`, both mode 600, and `updater.key.pub`. Back up both private files together to
the password manager now; losing either means no further update can be signed for shipped builds.

Rotation cannot be done by an update signed with the new key alone: installed apps trust only the
pinned public key. Sequence: generate a new pair (`npx tauri signer generate`), ship one release
signed with the OLD key whose `tauri.direct.conf.json` carries the NEW public key, then sign every
later release with the new key. If the old key is lost or compromised, existing installs cannot be
updated in place and users must install a fresh DMG.
