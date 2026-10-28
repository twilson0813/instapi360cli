//! Client configuration: which cloud environment and how the client identifies
//! itself. Values that are still being reverse-engineered are marked below and
//! centralized here so a single edit wires them in once capture confirms them.

use std::fmt;

/// Cloud data region.
///
/// The account/app API lives on a single global host (`openapi-g.insta360.com`
/// for OverSea), but the **cloud media** API (`/cloud/service/*`) is served from
/// a *data-center-specific* `service-*` host, and a request only succeeds on the
/// host that owns the account's data — the others reject with
/// `403 "没有权益"` ("no entitlement"). The correct host is selected by the
/// account's data region, which the session JWT carries as `userRegion` and the
/// subscription API returns as `userRegion: "US"` etc.
///
/// Verified mappings (captured from Insta360 Studio 6.0.6, macOS):
/// - US / Americas  → `service-va.insta360.com`   (Virginia); media CDN
///   `cloud-media-vaf.insta360.com`.
/// - EU / OverSea   → `service-fra.insta360.com`  (Frankfurt).
/// - APAC/Singapore → `service-sg.insta360.com`.
/// - Mainland China → `service.insta360.com`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Region {
    /// US / Americas (`service-va`). The JWT's `userRegion: 2` maps here.
    #[default]
    Us,
    /// EU / OverSea (`service-fra`).
    Eu,
    /// APAC / Singapore (`service-sg`).
    Apac,
    /// Mainland China (`service.insta360.com`).
    Cn,
    /// Test environment.
    Test,
}

impl Region {
    /// Map the session JWT's `userRegion` integer to a [`Region`].
    ///
    /// Observed: `2` = US (this account, served from `service-va`). Other codes
    /// are mapped on a best-effort basis and can be overridden by the caller.
    pub fn from_user_region(code: i64) -> Option<Region> {
        match code {
            2 => Some(Region::Us),
            1 => Some(Region::Eu),
            3 => Some(Region::Apac),
            0 => Some(Region::Cn),
            _ => None,
        }
    }

    /// Base origin for the **account/app** API (`/account/*`, `/app/*`). This is
    /// a single global host for all OverSea regions; only CN differs.
    pub fn openapi_base(self) -> &'static str {
        match self {
            Region::Cn => "https://openapi.insta360.com",
            Region::Test => "https://openapi-test.insta360.com",
            // US/EU/APAC all use the global account host.
            _ => "https://openapi-g.insta360.com",
        }
    }

    /// Base origin for the **cloud media** API (`/cloud/service/*`) — the
    /// data-center host that owns the account's media.
    pub fn cloud_base(self) -> &'static str {
        match self {
            Region::Us => "https://service-va.insta360.com",
            Region::Eu => "https://service-fra.insta360.com",
            Region::Apac => "https://service-sg.insta360.com",
            Region::Cn => "https://service.insta360.com",
            Region::Test => "https://service-test.insta360.com",
        }
    }

    /// Value sent in the `X-MainLand` header (`1` for CN, `0` otherwise).
    pub fn mainland_flag(self) -> &'static str {
        match self {
            Region::Cn => "1",
            _ => "0",
        }
    }
}

/// The platform value sent in `x-insta360-platform` and used to pick the
/// client-identity constants. The desktop app sends `windows`; a future Android
/// build sends `android`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Insta360 Studio on macOS — sends `pc/mac`.
    Mac,
    /// Insta360 Studio on Windows — sends `pc/windows`.
    Windows,
    /// Mobile app — sends `android`.
    Android,
}

impl Platform {
    /// Value for `x-insta360-platform`. The cloud media API validates this
    /// against the host: `service-va` only accepts the desktop values
    /// (`pc/mac` / `pc/windows`) and rejects `web`/`ios`/`android` with
    /// `code -2 "缺少正确的头部参数"` ("missing correct header parameter").
    pub fn header_value(self) -> &'static str {
        match self {
            Platform::Mac => "pc/mac",
            Platform::Windows => "pc/windows",
            Platform::Android => "android",
        }
    }

    /// The platform matching the current build's OS. Picking this by target OS
    /// matters: the media API ties `x-insta360-platform` to a desktop client,
    /// so a macOS host must present `pc/mac`.
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            // Linux and others: present as the Windows desktop client, which the
            // media API accepts.
            Platform::Windows
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.header_value())
    }
}

/// Static client identity + request-signing material.
///
/// NOTE (reverse-engineering pending): `app_key`, `client_id` and `sign_key`
/// are not hardcoded literals in the desktop binary — they are recovered from
/// mitmproxy capture / APK analysis (see the project plan, Steps 2–3). Until
/// then they default to empty and signing will return [`crate::Error::Signing`].
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub region: Region,
    pub platform: Platform,
    /// `X-APP-VERSION` — e.g. the Studio build `6.0.2` or the app version.
    pub app_version: String,
    /// `X-APP` — short app identifier.
    pub app_id: String,
    /// `app_traffic` header — release channel (e.g. `release`).
    pub app_traffic: String,
    /// `X-Client-Id`.
    pub client_id: String,
    /// `X-MCS-AppKey` — the app key sent with signed requests.
    pub app_key: String,
    /// Per-install `X-Equipment-Code`. Persisted by the caller across runs.
    pub equipment_code: String,
    /// UI language sent as `X-Language` (e.g. `en`).
    pub language: String,
    /// HMAC key for request signing (`client_key`/secret). Pending capture.
    pub sign_key: String,
    /// `User-Agent` header.
    pub user_agent: String,
}

impl ClientConfig {
    /// Config for a desktop Studio client on the given `platform`.
    ///
    /// Identity values (`app_version`, `app_traffic`, `user_agent`) mirror the
    /// live Studio 6.0.6 requests captured from the macOS client. Request
    /// signing is **not** used by the account/media/download endpoints — the
    /// session token alone authenticates — so `app_key`/`sign_key` stay blank
    /// (only the share-link endpoints HMAC-sign).
    pub fn for_platform(region: Region, platform: Platform) -> Self {
        let app_id = match platform {
            Platform::Mac => "studio_mac",
            Platform::Windows => "studio_win",
            Platform::Android => "android",
        };
        ClientConfig {
            region,
            platform,
            app_version: "6.0.6".to_string(),
            app_id: app_id.to_string(),
            app_traffic: "release".to_string(),
            client_id: String::new(),
            app_key: String::new(),
            equipment_code: String::new(),
            language: "en".to_string(),
            sign_key: String::new(),
            user_agent: "insta360/studio".to_string(),
        }
    }

    /// Config for the macOS Studio client (`pc/mac`).
    pub fn mac(region: Region) -> Self {
        Self::for_platform(region, Platform::Mac)
    }

    /// Config for the Windows Studio client (`pc/windows`).
    pub fn windows(region: Region) -> Self {
        Self::for_platform(region, Platform::Windows)
    }

    /// True once the fields required to sign an API request are populated.
    pub fn can_sign(&self) -> bool {
        !self.app_key.is_empty() && !self.sign_key.is_empty()
    }
}
