# Changes relative to upstream

This repository continues [sunsided/instapi360](https://github.com/sunsided/instapi360).
The upstream project had the right architecture (token auth, envelope parsing,
resumable OSS downloads) but its cloud endpoints were configured against the
wrong data-center host and client platform, so `list`/`download` failed with a
misleading `403 "没有权益"` ("no entitlement"). These changes make it work
against live accounts and add quality-of-life features.

## Root-cause fix: region + platform

The cloud media API (`/cloud/service/*`) is served from a **region-specific
host**, and a request only succeeds on the host that owns the account's data;
other regional hosts reject with `403 "没有权益"`. The correct host is implied by
the session token's `userRegion` claim.

- **`config.rs`**
  - Replaced the old `Region { Global, Cn, Test }` with
    `Region { Us, Eu, Apac, Cn, Test }` mapped to the correct `service-*` hosts
    (`service-va` US, `service-fra` EU, `service-sg` APAC, `service` CN).
  - Added `Region::from_user_region()` to map the token's `userRegion`
    (`2`→US, `1`→EU, `3`→APAC, `0`→CN).
  - Added `Platform::Mac` (`pc/mac`) and `Platform::current()` to pick the
    platform header from the host OS; the media API validates this value and
    rejects `web`/`ios`/`android` on these hosts.
  - Added `ClientConfig::for_platform()` / `mac()`; identity values
    (`app_version` etc.) aligned to the captured Studio 6.0.6 client.

- **`main.rs` (CLI)**
  - `--region auto` (default) detects the region from the stored token's
    `userRegion`; `us|eu|apac|cn|test` override it.
  - Platform is chosen by host OS automatically.

- **`store.rs`** — equipment-code auto-detection now has a macOS/BSD `ifconfig`
  fallback (was Linux sysfs only), so `X-Equipment-Code` is populated on macOS.

## New feature: shoot-date range filtering

- `list` and `download all` accept `--since` / `--until`, as `YYYY-MM-DD`
  (UTC, inclusive at both ends) or a relative age (`7d`, `12h`, `3w`).
- Filtering uses the API's **`shootTime`** (capture time), not `createTime`
  (upload time), so ranges match the clip filenames. `client.rs` now prefers
  `shootTime` when building a media item's timestamp.
- `list` output shows each item's shoot date.

## New feature: dry run

- `download all --dry-run` resolves and prints each matching item with its real
  size and a grand total, without downloading — useful for sizing a target
  (e.g. a Samba share) before a large pull.

## Behaviour: oldest → newest

- `download all` now downloads in ascending shoot-time order, so a long run
  fills in chronologically.

## New: interactive bash UI

- `tools/insta360cli-ui.sh` — a menu wrapper over the CLI (whoami, list, set
  date range, set output dir, dry-run estimate, download with resume, import
  token). Locates the binary on `PATH`, next to itself, or via `INSTAPI360_BIN`.

## Diagnostics

- `-v/--verbose` logs each API request (headers, token redacted) and the raw
  response body.
- `tools/capture_insta360.py` — a mitmproxy addon used to capture a real Studio
  session for reverse-engineering (redacts auth headers). Kept as a dev aid;
  the capture output it writes is git-ignored.

## Docs

- README documents the region/platform model, the date semantics, the bash UI,
  and a troubleshooting section (403 region mismatch, empty date ranges,
  `command not found`, etc.).
