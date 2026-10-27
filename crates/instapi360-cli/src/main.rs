//! `instapi360` — CLI over `instapi360-cloud`: import a session token, inspect the
//! account, list cloud media, and download originals.
//!
//! Session model: import a token once (from the app you're signed into), then
//! the session renews itself headlessly via `/account/v2/refreshToken` — no
//! repeated login. Credential `login` is blocked by reCAPTCHA and is a stub.

#[cfg(feature = "login")]
mod login;
mod store;

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use instapi360_cloud::{
    Client, ClientConfig, MediaId, PageCursor, Platform, Region, Session, SessionStore,
};
use store::{AppConfig, FileSessionStore};
use tokio::io::BufWriter;

#[derive(Parser)]
#[command(
    name = "insta360cli",
    version,
    about = "List and download original media from the Insta360 cloud"
)]
struct Cli {
    /// Cloud data region. `auto` (default) picks it from the token's
    /// `userRegion`. Override only if detection is wrong.
    #[arg(long, value_enum, default_value = "auto", global = true)]
    region: RegionArg,

    /// Override the per-install equipment code (X-Equipment-Code).
    #[arg(long, env = "INSTA360_EQUIPMENT_CODE", global = true)]
    equipment_code: Option<String>,

    /// Verbose: log every API request (URL + headers) and raw response body.
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    cmd: Command,
}

#[derive(Copy, Clone, clap::ValueEnum)]
enum RegionArg {
    /// Detect the region from the stored token's `userRegion`.
    Auto,
    /// US / Americas (`service-va`).
    Us,
    /// EU / OverSea (`service-fra`).
    Eu,
    /// APAC / Singapore (`service-sg`).
    Apac,
    Cn,
    Test,
}

impl RegionArg {
    /// Resolve to a concrete [`Region`], using the session token for `Auto`.
    fn resolve(self, session: Option<&Session>) -> Region {
        match self {
            RegionArg::Us => Region::Us,
            RegionArg::Eu => Region::Eu,
            RegionArg::Apac => Region::Apac,
            RegionArg::Cn => Region::Cn,
            RegionArg::Test => Region::Test,
            RegionArg::Auto => session
                .and_then(|s| instapi360_cloud::jwt_user_region(&s.user_token))
                .and_then(Region::from_user_region)
                .unwrap_or_default(),
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Store a session token (and optional refresh token) from the app, so the
    /// CLI can act headlessly and renew the session without re-login.
    ImportToken {
        /// The session/access token value.
        token: String,
        /// Optional refresh token, to renew the session via `refresh`.
        #[arg(long)]
        refresh_token: Option<String>,
    },
    /// Renew the stored session (mints a fresh token from the current one).
    Refresh,
    /// Interactive login: open a browser, sign in, capture the session token.
    /// (Requires the `login` feature — on by default.)
    Login,
    /// Show the authenticated account profile.
    Whoami,
    /// List cloud media.
    List {
        /// Page size.
        #[arg(long, default_value_t = 50)]
        count: u32,
        /// Fetch and print all pages, not just the first.
        #[arg(long)]
        all: bool,
        /// Only items captured on/after this date. Accepts `YYYY-MM-DD` or a
        /// relative age like `7d`, `12h`, `3w`.
        #[arg(long)]
        since: Option<String>,
        /// Only items captured on/before this date (`YYYY-MM-DD` or relative).
        #[arg(long)]
        until: Option<String>,
    },
    /// Download directly from a resolved CDN resourceKey URL (bypasses the API).
    /// Useful today: paste a SOURCE_URL from the app's download.db.
    DownloadUrl {
        /// The full https URL including `?resourceKey=...`.
        url: String,
        /// Output directory.
        #[arg(long, default_value = "./footage")]
        out: PathBuf,
        /// Resume a partial file.
        #[arg(long)]
        resume: bool,
    },
    /// Download a media item (or `all`) to a directory.
    Download {
        /// A `mediaId`, or the literal `all`.
        target: String,
        /// Output directory.
        #[arg(long, default_value = "./footage")]
        out: PathBuf,
        /// Resume partial files instead of re-downloading.
        #[arg(long)]
        resume: bool,
        /// With `all`: only items captured on/after this date (`YYYY-MM-DD` or
        /// a relative age like `7d`, `12h`, `3w`).
        #[arg(long)]
        since: Option<String>,
        /// With `all`: only items captured on/before this date.
        #[arg(long)]
        until: Option<String>,
        /// List what would be downloaded (with per-item size) and the total,
        /// without downloading anything.
        #[arg(long)]
        dry_run: bool,
    },
}

/// An inclusive capture-time window in unix seconds; `None` ends are open.
#[derive(Clone, Copy, Default)]
struct DateRange {
    since: Option<i64>,
    until: Option<i64>,
}

impl DateRange {
    /// Parse `--since`/`--until` strings into a range. Accepts absolute
    /// `YYYY-MM-DD` (interpreted in UTC; `since` = start of day, `until` = end
    /// of day so the day is inclusive) or a relative age `<n>[hdw]` meaning "n
    /// hours/days/weeks ago".
    fn parse(since: &Option<String>, until: &Option<String>) -> Result<DateRange> {
        Ok(DateRange {
            since: since
                .as_deref()
                .map(|s| parse_when(s, false))
                .transpose()?,
            until: until.as_deref().map(|s| parse_when(s, true)).transpose()?,
        })
    }

    fn is_open(&self) -> bool {
        self.since.is_none() && self.until.is_none()
    }

    /// Whether a media item's capture time falls in the window. Items with no
    /// known time are kept only when the range is open.
    fn contains(&self, time_unix: Option<i64>) -> bool {
        match time_unix {
            Some(t) => self.since.map(|s| t >= s).unwrap_or(true)
                && self.until.map(|u| t <= u).unwrap_or(true),
            None => self.is_open(),
        }
    }

    fn describe(&self) -> String {
        let f = |t: i64| {
            time::OffsetDateTime::from_unix_timestamp(t)
                .ok()
                .and_then(|d| {
                    d.format(&time::format_description::well_known::Rfc3339)
                        .ok()
                })
                .unwrap_or_else(|| t.to_string())
        };
        match (self.since, self.until) {
            (Some(s), Some(u)) => format!("{} … {}", f(s), f(u)),
            (Some(s), None) => format!("since {}", f(s)),
            (None, Some(u)) => format!("until {}", f(u)),
            (None, None) => "all dates".to_string(),
        }
    }
}

/// Parse one date expression to unix seconds. `end_of_day` extends a bare
/// `YYYY-MM-DD` to 23:59:59 so an `--until` day is inclusive.
fn parse_when(s: &str, end_of_day: bool) -> Result<i64> {
    let s = s.trim();
    // Relative age: <n>[h|d|w].
    if let Some(secs) = parse_relative(s) {
        return Ok(now_unix() - secs);
    }
    // Absolute YYYY-MM-DD (UTC).
    let fmt = time::macros::format_description!("[year]-[month]-[day]");
    let date = time::Date::parse(s, &fmt)
        .with_context(|| format!("invalid date '{s}': use YYYY-MM-DD or a relative age like 7d"))?;
    let t = if end_of_day {
        time::Time::from_hms(23, 59, 59).unwrap()
    } else {
        time::Time::MIDNIGHT
    };
    Ok(date.with_time(t).assume_utc().unix_timestamp())
}

/// `30d` → seconds in 30 days, `12h`, `3w`. Returns `None` if not this shape.
fn parse_relative(s: &str) -> Option<i64> {
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = num.parse().ok()?;
    let mult = match unit {
        "h" => 3600,
        "d" => 86_400,
        "w" => 604_800,
        _ => return None,
    };
    Some(n * mult)
}

fn config_dir() -> Result<PathBuf> {
    let pd = ProjectDirs::from("com", "instapi360", "instapi360")
        .ok_or_else(|| anyhow!("cannot determine config directory"))?;
    Ok(pd.config_dir().to_path_buf())
}

fn build_config(region: Region, equipment_code: String) -> ClientConfig {
    // Platform is picked by the host OS: the media API ties
    // `x-insta360-platform` to a desktop client (`pc/mac` on macOS), and a
    // mismatched value is rejected before the request is even considered.
    let mut cfg = ClientConfig::for_platform(region, Platform::current());
    cfg.equipment_code = equipment_code;
    cfg
}

/// Build a client for the resolved region, given the stored session (used to
/// auto-detect region from the token's `userRegion` when `--region auto`).
fn build_client(cli: &Cli, equipment_code: String, session: Option<&Session>) -> Result<Client> {
    let region = cli.region.resolve(session);
    Client::new(build_config(region, equipment_code)).context("building client")
}

/// Resolve the equipment code: explicit flag/env wins (and is persisted);
/// otherwise a saved value; otherwise the auto-detected host MAC.
fn resolve_equipment_code(cli: &Cli, cfg_path: &std::path::Path) -> String {
    let mut app = AppConfig::load(cfg_path);
    if let Some(ec) = &cli.equipment_code {
        if app.equipment_code.as_deref() != Some(ec) {
            app.equipment_code = Some(ec.clone());
            let _ = app.save(cfg_path);
        }
        return ec.clone();
    }
    if let Some(ec) = app.equipment_code.clone() {
        return ec;
    }
    let detected = store::detect_equipment_code().unwrap_or_default();
    if !detected.is_empty() {
        app.equipment_code = Some(detected.clone());
        let _ = app.save(cfg_path);
    }
    detected
}

fn load_session(store: &FileSessionStore) -> Result<Session> {
    store
        .load()?
        .ok_or_else(|| anyhow!("no session — run `insta360cli import-token <TOKEN>` first"))
}

/// Seconds before expiry at which we proactively renew (3 days).
const REFRESH_MARGIN_SECS: i64 = 3 * 24 * 3600;

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A human note like " (expires in 29 days)" from a unix-seconds expiry.
fn expiry_note(exp: i64) -> String {
    let days = (exp - now_unix()) / 86_400;
    if days < 0 {
        " (expired)".to_string()
    } else {
        format!(" (expires in {days} days)")
    }
}

/// Load the session and renew it if it is missing an expiry or within the
/// refresh margin, persisting the fresh token. Keeps the CLI usable without the
/// user ever manually refreshing.
async fn ensure_fresh(client: &Client, store: &FileSessionStore) -> Result<Session> {
    let session = load_session(store)?;
    let due = session
        .expires_at
        .map(|e| now_unix() >= e - REFRESH_MARGIN_SECS)
        .unwrap_or(true);
    if !due {
        return Ok(session);
    }
    match client.refresh(&session).await {
        Ok(renewed) => {
            store.save(&renewed)?;
            Ok(renewed)
        }
        // If renewal fails but the token is still valid, proceed with it.
        Err(e) if session.expires_at.map(|x| now_unix() < x).unwrap_or(true) => {
            eprintln!("warning: token refresh failed ({e}); using existing token");
            Ok(session)
        }
        Err(e) => Err(e).context("session expired and refresh failed — re-import a token"),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let default_filter = if cli.verbose {
        "insta360cli=debug,instapi360_cli=debug,instapi360_cloud=debug"
    } else {
        "insta360cli=info,instapi360_cli=info"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let dir = config_dir()?;
    let store = FileSessionStore::new(dir.join("session.json"));
    let equipment_code = resolve_equipment_code(&cli, &dir.join("config.json"));
    // Build the client against the region implied by the stored token (so
    // `--region auto` points at the right data center). Commands that don't
    // need a session (e.g. import-token) tolerate its absence.
    let existing = store.load().ok().flatten();
    let client = build_client(&cli, equipment_code, existing.as_ref())?;

    match &cli.cmd {
        Command::ImportToken {
            token,
            refresh_token,
        } => {
            let mut session = Session::from_token(token.trim());
            session.refresh_token = refresh_token.as_ref().map(|s| s.trim().to_string());
            session.expires_at = instapi360_cloud::jwt_exp(&session.user_token);
            store.save(&session)?;
            println!(
                "Token stored at {}{}",
                store.path().display(),
                session.expires_at.map(expiry_note).unwrap_or_default()
            );
        }

        Command::Refresh => {
            let session = load_session(&store)?;
            let renewed = client
                .refresh(&session)
                .await
                .context("refreshing session")?;
            store.save(&renewed)?;
            println!(
                "Session refreshed{}",
                renewed.expires_at.map(expiry_note).unwrap_or_default()
            );
        }

        Command::Login => {
            #[cfg(feature = "login")]
            {
                login::run(&store).await?;
            }
            #[cfg(not(feature = "login"))]
            {
                return Err(anyhow!(
                    "This is the lean (no-browser) build, so there is no `login` \
                     command. Sign in by importing your session token instead:\n\n  \
                     1. Sign in at https://cloud.insta360.com in your browser\n  \
                     2. Open DevTools → Application → Cookies → cloud.insta360.com\n  \
                     3. Copy the value of the `insta_access_token` cookie\n  \
                     4. Run:  insta360cli import-token \"<that value>\"\n\n\
                     The token auto-renews, so this is a once-in-a-while step."
                ));
            }
        }

        Command::Whoami => {
            let session = ensure_fresh(&client, &store).await?;
            let p = client.profile(&session).await.context("fetching profile")?;
            println!(
                "user_id: {}\nemail:   {}\nname:    {}",
                p.user_id.unwrap_or_default(),
                p.email.unwrap_or_default(),
                p.name.unwrap_or_default(),
            );
        }

        Command::List {
            count,
            all,
            since,
            until,
        } => {
            let range = DateRange::parse(since, until)?;
            if !range.is_open() {
                eprintln!("filtering: {}", range.describe());
            }
            let session = ensure_fresh(&client, &store).await?;
            let mut cursor = PageCursor::first(*count);
            let mut shown = 0u64;
            loop {
                let page = client
                    .list_media(&session, cursor.clone())
                    .await
                    .context("listing media")?;
                for m in &page.items {
                    if !range.contains(m.time_unix) {
                        continue;
                    }
                    shown += 1;
                    println!(
                        "{:<28}  {:>10}  {:>12}  {:>2} part(s)  {}",
                        m.id,
                        fmt_date(m.time_unix),
                        human_size(m.size_original),
                        m.parts.len().max(1),
                        m.name
                    );
                }
                match (all, page.next) {
                    (true, Some(next)) => cursor = next,
                    _ => break,
                }
            }
            eprintln!("{shown} item(s) listed");
        }

        Command::DownloadUrl { url, out, resume } => {
            use instapi360_cloud::FilePart;
            tokio::fs::create_dir_all(out).await?;
            let file_name = url
                .split('?')
                .next()
                .and_then(|u| u.rsplit('/').next())
                .unwrap_or("download.bin")
                .to_string();
            // HEAD for size so the progress bar is accurate.
            let size = client
                .http()
                .head(url)
                .send()
                .await
                .ok()
                .and_then(|r| r.content_length())
                .unwrap_or(0);
            let part = FilePart {
                file_name: sanitize(&file_name),
                size,
                url: url.clone(),
                md5: None,
            };
            let dest = out.join(&part.file_name);
            let from = if *resume {
                tokio::fs::metadata(&dest)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0)
            } else {
                0
            };
            let pb = ProgressBar::new(part.size.max(1));
            pb.set_style(
                ProgressStyle::with_template(
                    "{msg:<32} [{bar:32}] {bytes:>10}/{total_bytes:<10} {bytes_per_sec}",
                )
                .unwrap()
                .progress_chars("=>-"),
            );
            pb.set_message(part.file_name.clone());
            pb.set_position(from);
            let file = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(from > 0)
                .truncate(from == 0)
                .open(&dest)
                .await?;
            let writer = BufWriter::new(file);
            client
                .download_part(&part, writer, from, |done, total| {
                    if let Some(t) = total {
                        pb.set_length(t);
                    }
                    pb.set_position(done);
                })
                .await?;
            pb.finish();
            println!("Saved {}", dest.display());
        }

        Command::Download {
            target,
            out,
            resume,
            since,
            until,
            dry_run,
        } => {
            let range = DateRange::parse(since, until)?;
            let session = ensure_fresh(&client, &store).await?;

            // Collect the target items. For `all`, page the whole library and
            // apply the date filter; otherwise a single id (date filter N/A).
            let items: Vec<(MediaId, Option<i64>, u64, String)> = if target == "all" {
                if !range.is_open() {
                    eprintln!("filtering: {}", range.describe());
                }
                let mut cursor = PageCursor::first(100);
                let mut acc = Vec::new();
                loop {
                    let page = client.list_media(&session, cursor.clone()).await?;
                    let next = page.next.clone();
                    for m in page.items {
                        if range.contains(m.time_unix) {
                            acc.push((m.id, m.time_unix, m.size_original, m.name));
                        }
                    }
                    match next {
                        Some(n) => cursor = n,
                        None => break,
                    }
                }
                // Download oldest → newest by shoot time so a long `all` run
                // fills in chronologically. Items with no known time sort last.
                acc.sort_by_key(|(_, t, _, _)| t.unwrap_or(i64::MAX));
                acc
            } else {
                // Single id: look up the media name so the folder matches Studio
                // (best-effort — fall back to the id if detail is unavailable).
                let id = MediaId(target.clone());
                let name = client
                    .media_detail(&session, &id)
                    .await
                    .ok()
                    .map(|m| m.name)
                    .unwrap_or_default();
                vec![(id, None, 0, name)]
            };

            eprintln!("{} item(s) match", items.len());

            if *dry_run {
                // Resolve each item's real size so the total is accurate, then
                // print without downloading.
                let mut total: u64 = 0;
                for (id, time, _sz, name) in &items {
                    let parts = client.resolve_download(&session, id).await.ok();
                    let size: u64 = parts
                        .as_ref()
                        .map(|p| p.iter().map(|x| x.size).sum())
                        .unwrap_or(0);
                    total += size;
                    println!(
                        "{:<28}  {:>10}  {:>12}  {}",
                        id,
                        fmt_date(*time),
                        human_size(size),
                        name
                    );
                }
                println!("\nTotal: {} across {} item(s)", human_size(total), items.len());
                eprintln!("(dry run — nothing downloaded)");
                return Ok(());
            }

            tokio::fs::create_dir_all(out).await?;
            let mp = MultiProgress::new();
            let batch = target == "all";
            let total = items.len();
            let mut failed: Vec<(MediaId, String)> = Vec::new();
            for (n, (id, _time, _sz, name)) in items.into_iter().enumerate() {
                let name_opt = (!name.is_empty()).then_some(name.as_str());
                match download_one(&client, &session, &id, name_opt, out, *resume, &mp).await {
                    Ok(()) => {}
                    Err(e) => {
                        // In a single-item download, fail hard. In `all`, record
                        // and keep going so one bad item can't abort the batch.
                        if !batch {
                            return Err(e).with_context(|| format!("downloading {id}"));
                        }
                        eprintln!("[{}/{total}] skipping {id}: {e:#}", n + 1);
                        failed.push((id, format!("{e:#}")));
                    }
                }
            }
            if !failed.is_empty() {
                eprintln!("\n{} item(s) failed after retries:", failed.len());
                for (id, why) in &failed {
                    eprintln!("  {id}  —  {why}");
                }
                eprintln!("Re-run the same command to retry just these (completed files are skipped).");
            }
        }
    }

    Ok(())
}

/// Max attempts per file part before giving up (each resumes from bytes on disk).
const MAX_PART_ATTEMPTS: u32 = 6;

async fn download_one(
    client: &Client,
    session: &Session,
    id: &MediaId,
    name: Option<&str>,
    out: &std::path::Path,
    resume: bool,
    mp: &MultiProgress,
) -> Result<()> {
    let mut parts = client.resolve_download(session, id).await?;
    // One logical asset -> its own subdirectory, so multi-part clips stay
    // grouped. Name it like Insta360 Studio does: the media's base name with an
    // `-Original` suffix (e.g. `VID_20261016_140222_00_001-Original`). Fall back
    // to the mediaId when the name is unknown (e.g. a bare `download <id>`).
    let asset_dir = out.join(asset_folder_name(id, name));
    tokio::fs::create_dir_all(&asset_dir).await?;

    for idx in 0..parts.len() {
        let dest = asset_dir.join(sanitize(&parts[idx].file_name));

        let pb = mp.add(ProgressBar::new(parts[idx].size.max(1)));
        pb.set_style(
            ProgressStyle::with_template(
                "{msg:<32} [{bar:32}] {bytes:>10}/{total_bytes:<10} {bytes_per_sec}",
            )
            .unwrap()
            .progress_chars("=>-"),
        );
        pb.set_message(parts[idx].file_name.clone());

        // Retry loop: resume from whatever is already on disk. On a transient
        // network error we back off and retry; on an expired OSS URL (403) we
        // re-resolve to get fresh signed URLs, then continue from the same
        // offset. The partial file on disk is the durable checkpoint.
        let mut attempt = 0u32;
        loop {
            // Always resume from the current on-disk length (even when `resume`
            // is false, a mid-download retry must continue, not truncate).
            let on_disk = tokio::fs::metadata(&dest).await.map(|m| m.len()).unwrap_or(0);
            let from = if attempt == 0 && !resume { 0 } else { on_disk };

            if parts[idx].size > 0 && from >= parts[idx].size {
                break; // already complete
            }
            pb.set_position(from);

            let file = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(from > 0)
                .truncate(from == 0)
                .open(&dest)
                .await?;
            let writer = BufWriter::new(file);

            let res = client
                .download_part(&parts[idx], writer, from, |done, total| {
                    if let Some(t) = total {
                        pb.set_length(t);
                    }
                    pb.set_position(done);
                })
                .await;

            match res {
                Ok(()) => break,
                Err(e) => {
                    attempt += 1;
                    if attempt >= MAX_PART_ATTEMPTS {
                        pb.abandon_with_message(format!("{} — failed", parts[idx].file_name));
                        return Err(e).with_context(|| {
                            format!("part {} after {attempt} attempts", parts[idx].file_name)
                        });
                    }
                    // Expired signed URL -> re-resolve the whole asset's URLs.
                    if e.is_auth() {
                        if let Ok(fresh) = client.resolve_download(session, id).await {
                            if fresh.len() == parts.len() {
                                parts = fresh;
                            }
                        }
                    }
                    let backoff = std::time::Duration::from_secs(2u64.pow(attempt.min(4)));
                    pb.set_message(format!(
                        "{} — retry {attempt}/{MAX_PART_ATTEMPTS} in {}s",
                        parts[idx].file_name,
                        backoff.as_secs()
                    ));
                    tokio::time::sleep(backoff).await;
                }
            }
        }
        pb.finish();
    }
    println!("Saved {} part(s) to {}", parts.len(), asset_dir.display());
    Ok(())
}

/// Asset folder name derived from the media name: the capture stem
/// `PREFIX_<date>_<time>` with the trailing lens/chapter suffix and extension
/// dropped, e.g. `VID_20261016_140222_00_001.insv` -> `VID_20261016_140222`.
/// This stays stable across single- and multi-chapter clips and reads by
/// date/time. Falls back to the whole name (ext stripped), then the mediaId.
fn asset_folder_name(id: &MediaId, name: Option<&str>) -> String {
    let cleaned = name.map(|n| n.trim()).filter(|n| !n.is_empty());
    let folder = cleaned
        .and_then(capture_stem)
        .or_else(|| cleaned.map(strip_media_ext))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| id.0.clone());
    sanitize(&folder)
}

/// Strip a known media extension (case-insensitive) from a filename.
fn strip_media_ext(n: &str) -> String {
    let lower = n.to_ascii_lowercase();
    for ext in [".insv", ".insp", ".mp4", ".jpg", ".jpeg", ".dng", ".lrv"] {
        if lower.ends_with(ext) {
            return n[..n.len() - ext.len()].to_string();
        }
    }
    n.to_string()
}

/// Extract the `PREFIX_YYYYMMDD_HHMMSS` capture stem from a camera filename
/// like `VID_20261016_140222_00_001.insv` or `IMG_20260101_000000_00_001.insp`.
/// Returns `None` if the name doesn't match that shape.
fn capture_stem(n: &str) -> Option<String> {
    let parts: Vec<&str> = n.split('_').collect();
    // Need at least PREFIX, date, time (e.g. VID / 20261016 / 140222 ...).
    if parts.len() < 3 {
        return None;
    }
    let prefix = parts[0];
    let date = parts[1];
    // time segment may carry the extension if there's no lens suffix, e.g.
    // "VID_20261016_140222.insv" -> time piece "140222.insv".
    let time_raw = parts[2];
    let time = time_raw.split('.').next().unwrap_or(time_raw);
    let is_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if prefix.chars().all(|c| c.is_ascii_alphabetic())
        && date.len() == 8
        && is_digits(date)
        && time.len() == 6
        && is_digits(time)
    {
        Some(format!("{prefix}_{date}_{time}"))
    } else {
        None
    }
}

/// Format a unix-second capture time as `YYYY-MM-DD` (UTC), or `?` if unknown.
fn fmt_date(time_unix: Option<i64>) -> String {
    match time_unix {
        Some(t) => time::OffsetDateTime::from_unix_timestamp(t)
            .ok()
            .and_then(|d| {
                let fmt = time::macros::format_description!("[year]-[month]-[day]");
                d.format(&fmt).ok()
            })
            .unwrap_or_else(|| "?".to_string()),
        None => "?".to_string(),
    }
}

fn human_size(bytes: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut b = bytes as f64;
    let mut i = 0;
    while b >= 1024.0 && i < U.len() - 1 {
        b /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} {}", U[0])
    } else {
        format!("{b:.1} {}", U[i])
    }
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn folder_names() {
        let id = MediaId("123".into());
        let f = |n: Option<&str>| asset_folder_name(&id, n);
        assert_eq!(f(Some("VID_20260421_211837_00_003.insv")), "VID_20260421_211837");
        assert_eq!(f(Some("VID_20261016_140222_00_001.insv")), "VID_20261016_140222");
        assert_eq!(f(Some("IMG_20260101_000000_00_001.insp")), "IMG_20260101_000000");
        // no lens suffix, ext directly after time
        assert_eq!(f(Some("VID_20260101_120000.mp4")), "VID_20260101_120000");
        // unrecognized shape -> ext stripped, whole name kept
        assert_eq!(f(Some("weird-name.insv")), "weird-name");
        // empty / missing -> mediaId
        assert_eq!(f(None), "123");
        assert_eq!(f(Some("   ")), "123");
    }
}
