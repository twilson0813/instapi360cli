# insta360cli

A small Rust library (`instapi360-cloud`) and CLI (`insta360cli`) to access **your
own** Insta360 cloud media from the command line: sign in with your existing
session, list your uploads, and download the **original** camera files
(360 `.insv` + `.lrv` proxy, flat `.mp4`) for local editing and archival.

> **Based on [sunsided/instapi360](https://github.com/sunsided/instapi360)**
> by Markus Mayer, used under the MIT/Apache-2.0 license. This repository is an
> independent continuation that makes the cloud API work against live accounts
> (correct per-region host + platform selection), adds shoot-date range
> filtering, a size-estimating dry run, oldest-first ordering, and an
> interactive bash UI. See [CHANGES.md](CHANGES.md) for the full list.

> **Not affiliated with Insta360.** This project is an independent, unofficial
> tool. It is **not** affiliated with, endorsed by, sponsored by, or connected
> to Insta360 or Arashivision. "Insta360", "Ace Pro", "X5" and related names are
> trademarks of their respective owners and are used here only nominatively to
> describe interoperability.
>
> **Personal use only.** Intended solely for accessing **your own account** and
> **your own footage** that you have the right to access. You are responsible
> for complying with Insta360's Terms of Service and applicable law. Do not use
> it to access anyone else's data.

## Workspace
| crate | role |
|-------|------|
| `instapi360-cloud` | Async client library: auth, list, resolve, download. rustls-only, I/O injected by the caller (portable, no filesystem assumptions in the core). |
| `instapi360-cli`   | The `insta360cli` binary over the library. |

## Build & test
```sh
cargo build
cargo test -p instapi360-cloud
cargo run -p instapi360-cli -- --help
```

## Usage
```sh
# 1a. interactive login — opens a browser, you sign in (and solve any captcha),
#     the token is captured automatically  (needs Chrome/Chromium)
insta360cli login
# 1b. …or paste a token you already have
insta360cli import-token "<session-token>"

# 2. use it — equipment code auto-detects your host and is remembered
insta360cli whoami
insta360cli list --all
insta360cli download <mediaId> --out ./footage      # grabs all parts (.insv + .lrv)
insta360cli download all --out ./footage --resume

# 3. keep the session alive headlessly (no re-login)
insta360cli refresh        # or let commands auto-renew when near expiry
```

## Date-range filtering

`list` and `download all` take `--since` / `--until` to select media by **shoot
date** (when the footage was recorded — matches the filename, not the cloud
upload time). Dates are `YYYY-MM-DD` (UTC, inclusive) or a relative age:
`7d`, `12h`, `3w` (= "7 days ago", etc.).

```sh
# everything shot in October 2026
insta360cli list --all --since 2026-10-01 --until 2026-10-31

# download a single day's footage to a Samba share
insta360cli download all --out /Volumes/Share/Insta360 --resume \
  --since 2026-08-30 --until 2026-08-30

# everything from the last two weeks
insta360cli download all --out ./footage --resume --since 2w
```

`download all` downloads **oldest → newest** by shoot time, so a long run fills
in chronologically.

### Preview before a big pull
`--dry-run` lists exactly what would download, with per-item and total size
(resolved from the API), without downloading anything — handy for sizing a share:

```sh
insta360cli download all --since 2026-08-30 --until 2026-08-30 --dry-run --out ./x
# …
# Total: 94.7 GB across 10 item(s)
```

## Interactive menu (bash UI)

For a point-and-pick workflow, `insta360cli-ui` wraps the CLI with a menu:
who-am-I, list, set a date range, set the output directory (local or a mounted
Samba share), estimate size (dry run), and download with resume.

```sh
insta360cli-ui
```

It finds the `insta360cli` binary on `PATH`, next to the script, or at
`~/.cargo/bin/insta360cli`; override with `INSTA360CLI_BIN=/path/to/insta360cli`.
Set a default output dir with `INSTA360CLI_OUT=/Volumes/Share/Insta360`.
(The legacy `instapi360` binary name is still accepted.)

The `login` command is behind the **`login`** cargo feature, **on by default**.
It pulls a Chrome-automation dependency; for a lean, token-import-only build:
`cargo build --no-default-features`.

## Signing in

**Requirement:** Chrome or Chromium installed (used only for the login window;
the rest of the CLI needs no browser).

### Browser login (recommended)
```sh
insta360cli login
```
1. A **fresh** Chrome window opens on the Insta360 sign-in page. It uses a
   throwaway profile (removed afterwards), so it won't collide with a Chrome you
   already have running — and you sign in from scratch in this window.
2. Sign in with your account, solving any reCAPTCHA challenge that appears.
3. Once you're signed in, the CLI reads the session token from the page and
   stores it at `~/.config/instapi360/session.json`, then closes the browser.
   You'll see: `Logged in — token stored … (expires in 30 days)`.

The window waits up to 5 minutes for you to finish. Nothing is typed for you —
you enter your own credentials directly into Insta360's page.

### Manual token import (no browser)
If you already have a session token (e.g. from a build without the `login`
feature), store it directly:
```sh
insta360cli import-token "<session-token>"
```

### Staying signed in
The token lasts ~30 days and **renews itself**: `whoami`, `list` and `download`
auto-renew it when it's within a few days of expiring, and `insta360cli refresh`
renews on demand. So `login` is a once-in-a-while action, not per-use.

### Where the session is stored
The session JSON lives in your OS config dir:
- macOS: `~/Library/Application Support/com.instapi360.instapi360/session.json`
- Linux: `~/.config/instapi360/session.json`

Delete that file to sign out.

### Troubleshooting
- **`403 "没有权益"` ("no entitlement") on `list`/`download`** — almost always
  the wrong **region host** for your account (see "Region & platform"). Let
  `--region auto` detect it from your token, or set `--region` explicitly. It is
  *not* usually a subscription problem.
- **`list`/`download` find nothing for a date range** — filtering is by
  `shootTime`; confirm you actually have footage in that range with
  `insta360cli list --all` (dates are shown per item). A `--dry-run` also lists
  exactly what matches.
- **`command not found: insta360cli`** after installing — your shell cached the
  miss; run `hash -r` or open a new terminal.
- **`is a directory` from the bash UI** — point it at the binary explicitly:
  `INSTA360CLI_BIN=/path/to/insta360cli ./insta360cli-ui`.
- **`oneshot canceled` / no window** — usually an outdated Chrome-automation
  stack against a very new Chrome. Update the toolchain (`cargo update`) and
  ensure Chrome/Chromium is current.
- **Headless/server box** — `login` needs a display for the browser window. On a
  headless machine, run `login` on a desktop, copy `session.json` over, or use
  `import-token`.

## How it works
- **Sign in once, stay signed in.** Authentication is your session token alone
  (sent as a header) — no request signing. The token lives ~30 days and is
  renewed via `/account/v2/refreshToken`, which mints a fresh token from the
  current one (no separate refresh token). `whoami`/`list`/`download`
  auto-renew when the token is within a few days of expiring, so a single
  import keeps working indefinitely.
- The sign-in endpoint is reCAPTCHA-gated, so a headless credential POST won't
  work. The practical way to get a token is to read the **`insta_access_token`
  cookie** from a browser signed in at `cloud.insta360.com` and feed it to
  `import-token`. (The optional `login` command drives a real browser via the
  Chrome DevTools Protocol, but cookie import is simpler and always works.)
- Listing and download resolution are authenticated GET requests. Downloads
  resolve to short-lived CDN URLs and stream with **resumable, ranged**
  transfers; multi-file media (video + proxy, or dual-lens parts) are grouped
  and saved together as one asset.

### Region & platform (important)
The cloud media API (`/cloud/service/*`) is served from a **data-center host
tied to your account's region**, and a request only succeeds on the host that
owns your data — the others reject with `403 "没有权益"` ("no entitlement"),
which is misleading because it looks like a subscription problem. The region is
read from your session token's `userRegion` claim and maps to:

| Region | `userRegion` | cloud host |
|--------|:---:|------------|
| US / Americas | `2` | `service-va.insta360.com` |
| EU / OverSea | `1` | `service-fra.insta360.com` |
| APAC / Singapore | `3` | `service-sg.insta360.com` |
| Mainland China | `0` | `service.insta360.com` |

`--region auto` (the default) picks this from your token; override with
`--region us|eu|apac|cn` if detection is ever wrong.

The API also validates the `x-insta360-platform` header against a desktop
client value — the CLI sends `pc/mac` on macOS and `pc/windows` elsewhere,
chosen automatically by the host OS. Values like `web`/`ios`/`android` are
rejected on these hosts.

### Dates
Date filtering uses the item's **`shootTime`** (when the footage was recorded),
not its cloud-upload time, so ranges line up with the clip filenames.
`shootTime` is stored in UTC; near-midnight clips at the very edge of a range
may fall on the adjacent UTC day, so pad by a day if you need to be exact.

## Design
`instapi360-cloud` is deliberately dependency-light and platform-agnostic
(`rustls`, caller-provided `AsyncWrite` sinks and a `SessionStore` trait) so it
can be embedded in other apps — desktop or mobile — not just this CLI.

## Roadmap
- Media metadata (camera, timestamps, gyro/immersion data) surfaced in `list`.

## License
Licensed under either of MIT or Apache-2.0 at your option.

## Naming
`insta360cli` is a working name and may change; it is not an official product name.
