//! Every built-in address of an outside service vanilla doesn't contact, in one place
//! (docs/layers.md#built-in-addresses). `tools/check_addresses.py` keeps new ones from going
//! anywhere else.
//!
//! This is a Sunrise-layer crate: the engine and the platform never depend on it. Platform code
//! that talks to a service gets the address passed in from a Sunrise crate.

/// Sunrise's website, where the launcher is downloaded: the game started without its prepared
/// files sends the player there.
pub const WEBSITE: &str = "https://playsunrise.net";

/// The players' page on connecting an AI agent (the website's docs/agents.md).
pub const AGENT_DOCS: &str = "https://playsunrise.net/docs/agents";

/// The mod registry's policy, which the launcher's mod pages link.
pub const MOD_POLICY: &str = "https://playsunrise.net/mods/policy";

/// The mod registry's index, published by sunrise-org/sunrise-mods (`SUNRISE_MOD_INDEX` overrides it).
pub const MOD_INDEX: &str = "https://sunrise-org.github.io/sunrise-mods/index.json";

/// Where the registry's icons and screenshots are served from: the same GitHub Pages site.
pub const MOD_IMAGES: &str = "https://sunrise-org.github.io/sunrise-mods/";

/// A registry project's page on the website is `<this>/<id>`.
pub const MOD_PAGES: &str = "https://playsunrise.net/mods";

/// The registry's form for reporting a project.
pub const MOD_REPORT: &str =
    "https://github.com/sunrise-org/sunrise-mods/issues/new?template=report.yml";

/// The telemetry ingest server (`crates/sunrise-telemetry-server`): opt-in telemetry, the
/// "I'm playing" ping, benchmark uploads (`SUNRISE_TELEMETRY_URL` overrides it).
pub const TELEMETRY: &str = "https://telemetry.playsunrise.net";

/// The performance viewer page a saved capture opens in (`SUNRISE_PERF_VIEWER` overrides it).
pub const PERF_VIEWER: &str = "https://playsunrise.net/perf";

/// The capture upload API behind Share (`SUNRISE_PERF_CAPTURES` overrides it).
pub const PERF_CAPTURES: &str = "https://playsunrise.net/api/perf/captures";

/// Modrinth's API, which the in-game resource pack browser (`sunrise-pack-browser`) searches and
/// downloads from.
pub const MODRINTH_API: &str = "https://api.modrinth.com/v2";

/// Public STUN servers for Play with Friends' NAT discovery: two independent operators, so one
/// outage doesn't break discovery (`SUNRISE_STUN_SERVERS` overrides them).
pub const STUN_SERVERS: &[&str] = &["stun.cloudflare.com:3478", "stun.l.google.com:19302"];

/// Play with Friends' troubleshooting page, shown to hosts nobody can reach.
pub const FRIENDS_HELP: &str =
    "https://github.com/sunrise-org/sunrise/blob/main/docs/play-with-friends.md#troubleshooting";

/// Sunrise community invite used by launcher defaults.
pub const COMMUNITY_DISCORD: &str = "https://discord.gg/eswQcHfRKC";

/// The GitHub profile of who made the launcher's look, credited in Settings' About (its picture is
/// `<this>.png`).
pub const LAUNCHER_LOOK_CREDIT: &str = "https://github.com/Xmilon";

/// The official launcher installer's product home.
pub const LAUNCHER_ABOUT: &str = "https://playsunrise.net";

/// The separately branded arismoko fork launcher installer's product home.
pub const FORK_LAUNCHER_ABOUT: &str = "https://github.com/sunrise-org/sunrise";

/// Cargo's canonical source identity for the crates.io registry.
pub const CARGO_CRATES_IO_SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// The pinned horizon-mixins archive the launcher downloads for `[[native.mixin]]` builds.
pub const HORIZON_MIXINS_SOURCE: &str =
    "https://codeload.github.com/horizon-rust/horizon-mixins/tar.gz";

/// Where the fork releases the prebuilt horizon-mixins tool; a release's tag and file follow.
pub const MIXINS_TOOL_RELEASES: &str = "https://github.com/sunrise-org/sunrise/releases/download";

/// Where the fork publishes each game build's mod SDK (`sdk-<build id>`) and the game releases
/// whose `sdk.json` names their build; a release's tag and file follow. `SUNRISE_SDK_BASE`
/// overrides it.
pub const SDK_RELEASES: &str = "https://github.com/sunrise-org/sunrise/releases/download";

/// The fork's newest releases, which `cargo sunrise-mod` reads to find the one SDK a short sha names.
pub const SDK_RELEASE_LIST: &str =
    "https://api.github.com/repos/sunrise-org/sunrise/releases?per_page=100";
