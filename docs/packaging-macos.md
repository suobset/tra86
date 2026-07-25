# Packaging Bind for macOS (signing & notarization)

This guide covers building a distributable, **Developer ID-signed and notarized**
build of the `bind` CLI, and the macOS debugging-authorization model Bind
depends on at runtime.

> TL;DR: `scripts/package-macos.sh` builds a universal binary, signs it with the
> Hardened Runtime + Bind's entitlements, wraps it in a `.pkg`, and notarizes +
> staples it. Provide your own `SIGN_IDENTITY` / `INSTALLER_IDENTITY` and notary
> credentials.

## How debugging works on macOS (important)

Bind does not itself call `ptrace` / `task_for_pid`. It launches the LLDB
**SB-API Python driver**, and the privileged process control is performed by
Apple's `debugserver`, which Apple already ships signed with the
`com.apple.security.cs.debugger` entitlement. Two things must be true at runtime:

1. **The LLDB toolchain is installed** — Xcode or the Command Line Tools
   (`xcode-select --install`). Bind resolves it via `lldb -P` and `xcrun`.
2. **Debugging is authorized on the machine.** The first time a debugger
   attaches, macOS requires authorization. Enable it once (admin required):

   ```bash
   sudo DevToolsSecurity -enable
   ```

   Without this, live launch/attach blocks on a `taskgated` authorization
   prompt; Bind's request times out and the UI surfaces a clear error instead of
   hanging. Static inspection (symbols, disassembly) works regardless.

## Entitlements

`packaging/bind.entitlements` requests, under the Hardened Runtime:

| Entitlement | Why |
|---|---|
| `com.apple.security.cs.debugger` | Bind is a debugger; this permits it to attach to / control other processes under the Hardened Runtime. |
| `com.apple.security.cs.disable-library-validation` | The LLDB driver dlopens the LLDB framework from a runtime-resolved path. |
| `com.apple.security.cs.allow-dyld-environment-variables` | Bind passes `PYTHONPATH` to the child interpreter so it can import the LLDB SB-API module. |

`com.apple.security.get-task-allow` is **deliberately absent** — it is a
development-only entitlement and would fail notarization.

## Prerequisites

- An Apple Developer account and two certificates in your keychain:
  - **Developer ID Application** (signs the binary)
  - **Developer ID Installer** (signs the `.pkg`)
- Notary credentials, one of:
  - a stored notary profile (recommended):
    ```bash
    xcrun notarytool store-credentials bind-notary \
      --apple-id you@example.com --team-id TEAMID --password <app-specific-password>
    ```
  - or `APPLE_ID` / `TEAM_ID` / `APP_SPECIFIC_PASSWORD` env vars.
- Rust with both Apple targets (the script adds them):
  `aarch64-apple-darwin`, `x86_64-apple-darwin`.

## Build & notarize

```bash
# universal binary only (no signing) — good for a smoke check:
scripts/package-macos.sh --no-sign        # -> dist/bind (x86_64 arm64)

# signed .pkg, not notarized:
SIGN_IDENTITY="Developer ID Application: You (TEAMID)" \
INSTALLER_IDENTITY="Developer ID Installer: You (TEAMID)" \
  scripts/package-macos.sh --skip-notarize

# full: signed, notarized, stapled .pkg:
SIGN_IDENTITY="Developer ID Application: You (TEAMID)" \
INSTALLER_IDENTITY="Developer ID Installer: You (TEAMID)" \
NOTARY_PROFILE="bind-notary" \
  scripts/package-macos.sh
```

The script:

1. Builds `bind` for arm64 and x86_64 and `lipo`s them into a universal binary.
2. Signs it: `codesign --options runtime --timestamp --entitlements packaging/bind.entitlements`.
3. Builds a component package with `pkgbuild` (installs to `/usr/local/bin/bind`).
   A container format is required because a bare binary cannot be stapled.
4. `xcrun notarytool submit --wait`, then `xcrun stapler staple` the `.pkg`.

## Verifying a build

```bash
codesign --verify --strict --verbose=2 dist/bind
spctl --assess --type install --verbose=2 dist/bind-0.1.0.pkg   # after stapling
xcrun stapler validate dist/bind-0.1.0.pkg
```

## Notes & caveats

- **The `.pkg` does not bundle LLDB.** Bind uses the system LLDB; the target
  machine still needs Xcode/CLT and `sudo DevToolsSecurity -enable`.
- Distributing the bare binary in a `.zip` can be notarized but **not stapled**
  (only containers staple). Prefer the `.pkg` (or a `.dmg`) for offline Gatekeeper
  validation.
- A future Homebrew tap could wrap the notarized `.pkg`/binary; not provided yet.
- Bind is unsigned in normal `cargo build` development; signing matters only for
  distribution.
