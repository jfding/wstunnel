# Client TOML Config File Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users specify all wstunnel **client** settings (including multiple tunnels) from a TOML config file via `wstunnel client --config <FILE>`.

**Architecture:** Config-only mode. `Client` (in the `webtop` lib crate) gains a `serde::Deserialize` impl whose per-field `deserialize_with` helpers reuse the *existing* `parsers::*` functions, so the file accepts exactly the same strings the CLI accepts. A new `--config` flag conflicts with all other client args; when present, the CLI loads the `Client` from TOML instead of from flags.

**Tech Stack:** Rust (edition 2024), clap 4 (derive), serde 1 (derive), `toml` crate (new), tokio.

## Global Constraints

- Workspace: `wstunnel/` = lib crate `webtop` (config in `wstunnel/src/config.rs`); `wstunnel-cli/` = bin crate `webtop-cli` (`wstunnel-cli/src/main.rs`). The CLI depends on `webtop` with `default-features = false, features = ["clap"]`.
- `serde` is already a dependency with the `derive` feature. `toml` is NOT yet a dependency — add it.
- Changes are **client-only**. Do not modify the `Server` struct or server behavior.
- Reuse existing `parsers::*` functions — do NOT write a second parsing implementation for any format.
- TOML keys map 1:1 to `Client` struct field names (snake_case). `remote_addr` is the only required key; everything else has a default matching the current CLI default.
- `#[serde(deny_unknown_fields)]`: unknown/typo'd keys must be a hard error.
- Run all build/test commands from the repo root `/home/djf/code/devops/wstunnel`.
- Build/test the lib with the `clap` feature on (matches how the CLI consumes it): `cargo test -p webtop --features clap`.

---

### Task 1: Add `toml` dependency and make `parsers` module unconditional

The `serde` deserialize helpers (Task 2) call functions in `mod parsers`, which is currently gated behind `#[cfg(feature = "clap")]`. The `Deserialize` impl will be unconditional (serde is always available), so the parsers must be available without the `clap` feature too. The functions are pure (no clap types), so un-gating is safe.

**Files:**
- Modify: `wstunnel/Cargo.toml` (add `toml` dependency)
- Modify: `wstunnel/src/config.rs:411` (remove the `#[cfg(feature = "clap")]` on `mod parsers`)

**Interfaces:**
- Produces: `mod parsers` is compiled in all feature configurations; `toml` crate available to `webtop`.

- [ ] **Step 1: Add the `toml` dependency**

In `wstunnel/Cargo.toml`, in the `[dependencies]` section near the other serde/config crates (after the `serde = ...` line), add:

```toml
toml = { version = "0.9", default-features = false, features = ["parse"] }
```

(If `0.9` with `features = ["parse"]` fails to resolve, fall back to `toml = "0.8"` with no feature list — both expose `toml::from_str`.)

- [ ] **Step 2: Un-gate the `parsers` module**

In `wstunnel/src/config.rs`, find (around line 411):

```rust
#[cfg(feature = "clap")]
mod parsers {
```

Change it to:

```rust
mod parsers {
```

- [ ] **Step 3: Verify the lib builds with and without `clap`**

Run: `cargo build -p webtop --no-default-features --features aws-lc-rs`
Expected: compiles (warnings about unused `parsers` functions are acceptable here; Task 2 makes them used).

Run: `cargo build -p webtop --features clap`
Expected: compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add wstunnel/Cargo.toml wstunnel/src/config.rs
git commit -m "chore: add toml dep and make config parsers unconditional"
```

---

### Task 2: Add TOML deserialization to `Client`

Derive `serde::Deserialize` on `Client`, add per-field serde attributes (defaults + `deserialize_with` helpers that reuse `parsers::*`), add the free helper functions, and add `Client::from_config_file`.

**Files:**
- Modify: `wstunnel/src/config.rs` (imports at top; struct derive/attributes lines 11–248; add helpers + `impl Client`; add tests)

**Interfaces:**
- Consumes: `parsers::parse_tunnel_arg`, `parse_reverse_tunnel_arg`, `parse_duration_sec`, `parse_server_url`, `parse_sni_override`, `parse_http_credentials`, `parse_http_headers` — all `pub fn ...(&str) -> Result<T, std::io::Error>` (from Task 1, now always compiled).
- Produces:
  - `Client` implements `serde::Deserialize` with `deny_unknown_fields`.
  - `Client::from_config_file(path: &std::path::Path) -> anyhow::Result<Client>`.
  - New field `pub config: Option<PathBuf>` on `Client` (CLI-only; `#[serde(skip)]`).

- [ ] **Step 1: Write the failing tests**

Append this module to the **end** of `wstunnel/src/config.rs` (after the closing `}` of `mod parsers`):

```rust
#[cfg(test)]
mod config_file_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn minimal_applies_cli_defaults() {
        let toml = r#"
            remote_addr = "wss://server.example.com:443"
            local_to_remote = ["tcp://1212:google.com:443"]
        "#;
        let c: Client = toml::from_str(toml).expect("should parse");
        assert_eq!(c.remote_addr.as_str(), "wss://server.example.com/");
        assert_eq!(c.local_to_remote.len(), 1);
        assert_eq!(c.remote_to_local.len(), 0);
        // defaults matching the CLI
        assert_eq!(c.connection_min_idle, 0);
        assert_eq!(c.connection_retry_max_backoff, Duration::from_secs(300));
        assert_eq!(c.reverse_tunnel_connection_retry_max_backoff, Duration::from_secs(1));
        assert_eq!(c.http_upgrade_path_prefix, "v1");
        assert_eq!(c.websocket_ping_frequency, Some(Duration::from_secs(30)));
        assert!(!c.tls_verify_certificate);
        assert!(c.config.is_none());
    }

    #[test]
    fn multiple_tunnels_parse() {
        let toml = r#"
            remote_addr = "wss://server.example.com:443"
            local_to_remote = [
              "tcp://1212:google.com:443",
              "udp://1212:1.1.1.1:53?timeout_sec=10",
              "socks5://[::1]:1212",
            ]
            remote_to_local = ["tcp://1213:google.com:443"]
        "#;
        let c: Client = toml::from_str(toml).expect("should parse");
        assert_eq!(c.local_to_remote.len(), 3);
        assert_eq!(c.remote_to_local.len(), 1);
    }

    #[test]
    fn full_config_parses_values() {
        let toml = r#"
            remote_addr = "wss://server.example.com:443"
            local_to_remote = ["tcp://1212:google.com:443"]
            tls_verify_certificate = true
            connection_min_idle = 5
            connection_retry_max_backoff = "5m"
            websocket_ping_frequency = "10s"
            http_upgrade_path_prefix = "secret"
            http_headers = ["X-Foo: bar", "X-Baz: qux"]
            dns_resolver = ["dns://1.1.1.1"]
        "#;
        let c: Client = toml::from_str(toml).expect("should parse");
        assert!(c.tls_verify_certificate);
        assert_eq!(c.connection_min_idle, 5);
        assert_eq!(c.connection_retry_max_backoff, Duration::from_secs(300));
        assert_eq!(c.websocket_ping_frequency, Some(Duration::from_secs(10)));
        assert_eq!(c.http_upgrade_path_prefix, "secret");
        assert_eq!(c.http_headers.len(), 2);
        assert_eq!(c.dns_resolver.len(), 1);
    }

    #[test]
    fn unknown_key_is_rejected() {
        let toml = r#"
            remote_addr = "wss://server.example.com:443"
            bogus_key = 1
        "#;
        assert!(toml::from_str::<Client>(toml).is_err());
    }

    #[test]
    fn missing_remote_addr_is_rejected() {
        let toml = r#"
            local_to_remote = ["tcp://1212:google.com:443"]
        "#;
        assert!(toml::from_str::<Client>(toml).is_err());
    }

    #[test]
    fn malformed_tunnel_is_rejected() {
        let toml = r#"
            remote_addr = "wss://server.example.com:443"
            local_to_remote = ["this-is-not-a-tunnel"]
        "#;
        assert!(toml::from_str::<Client>(toml).is_err());
    }

    #[test]
    fn from_config_file_reads_and_parses() {
        let dir = std::env::temp_dir();
        let path = dir.join("webtop_test_from_config_file.toml");
        std::fs::write(
            &path,
            "remote_addr = \"wss://server.example.com:443\"\nlocal_to_remote = [\"tcp://1212:google.com:443\"]\n",
        )
        .unwrap();
        let c = Client::from_config_file(&path).expect("should load");
        assert_eq!(c.local_to_remote.len(), 1);
        let _ = std::fs::remove_file(&path);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p webtop --features clap config_file_tests`
Expected: FAIL — compile error, `Client` does not implement `Deserialize` / `from_config_file` not found.

- [ ] **Step 3: Add serde imports and helper functions**

At the **top** of `wstunnel/src/config.rs`, add to the existing `use` block (after `use std::time::Duration;`):

```rust
use serde::{Deserialize, Deserializer};
```

Then, immediately **before** the `#[derive(Clone, Debug)]` line for `pub struct Client` (around line 11, after the `DEFAULT_CLIENT_UPGRADE_PATH_PREFIX` const), add the helpers:

```rust
// ---- defaults for fields whose CLI default is not the type's zero value ----
fn default_connection_retry_max_backoff() -> Duration {
    Duration::from_secs(300) // "5m"
}
fn default_reverse_connection_retry_max_backoff() -> Duration {
    Duration::from_secs(1) // "1s"
}
fn default_http_upgrade_path_prefix() -> String {
    DEFAULT_CLIENT_UPGRADE_PATH_PREFIX.to_string()
}
fn default_websocket_ping_frequency() -> Option<Duration> {
    Some(Duration::from_secs(30)) // "30s"
}

// ---- deserializers that reuse the CLI parsers (single source of truth) ----
fn de_tunnels<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<LocalToRemote>, D::Error> {
    Vec::<String>::deserialize(d)?
        .into_iter()
        .map(|s| parsers::parse_tunnel_arg(&s).map_err(serde::de::Error::custom))
        .collect()
}
fn de_reverse_tunnels<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<LocalToRemote>, D::Error> {
    Vec::<String>::deserialize(d)?
        .into_iter()
        .map(|s| parsers::parse_reverse_tunnel_arg(&s).map_err(serde::de::Error::custom))
        .collect()
}
fn de_duration<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
    let s = String::deserialize(d)?;
    parsers::parse_duration_sec(&s).map_err(serde::de::Error::custom)
}
fn de_duration_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
    let s = String::deserialize(d)?;
    parsers::parse_duration_sec(&s).map(Some).map_err(serde::de::Error::custom)
}
fn de_server_url<'de, D: Deserializer<'de>>(d: D) -> Result<Url, D::Error> {
    let s = String::deserialize(d)?;
    parsers::parse_server_url(&s).map_err(serde::de::Error::custom)
}
fn de_url_vec<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Url>, D::Error> {
    Vec::<String>::deserialize(d)?
        .into_iter()
        .map(|s| Url::parse(&s).map_err(serde::de::Error::custom))
        .collect()
}
fn de_sni_override<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DnsName<'static>>, D::Error> {
    let s = String::deserialize(d)?;
    parsers::parse_sni_override(&s).map(Some).map_err(serde::de::Error::custom)
}
fn de_http_credentials<'de, D: Deserializer<'de>>(d: D) -> Result<Option<HeaderValue>, D::Error> {
    let s = String::deserialize(d)?;
    parsers::parse_http_credentials(&s).map(Some).map_err(serde::de::Error::custom)
}
fn de_http_headers<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Vec<(HeaderName, HeaderValue)>, D::Error> {
    Vec::<String>::deserialize(d)?
        .into_iter()
        .map(|s| parsers::parse_http_headers(&s).map_err(serde::de::Error::custom))
        .collect()
}
```

- [ ] **Step 4: Add the `Deserialize` derive and `deny_unknown_fields`**

Change the `Client` derive block (line 11–13) from:

```rust
#[derive(Clone, Debug)]
#[cfg_attr(feature = "clap", derive(clap::Args))]
pub struct Client {
```

to:

```rust
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "clap", derive(clap::Args))]
pub struct Client {
```

- [ ] **Step 5: Add serde attributes to each `Client` field**

Add a `#[serde(...)]` attribute line directly **above each field's existing** `#[cfg_attr(feature = "clap", arg(...))]` line. Use exactly these mappings:

| Field | serde attribute to add |
|---|---|
| `local_to_remote` | `#[serde(default, deserialize_with = "de_tunnels")]` |
| `remote_to_local` | `#[serde(default, deserialize_with = "de_reverse_tunnels")]` |
| `socket_so_mark` | `#[serde(default)]` |
| `connection_min_idle` | `#[serde(default)]` |
| `connection_retry_max_backoff` | `#[serde(default = "default_connection_retry_max_backoff", deserialize_with = "de_duration")]` |
| `reverse_tunnel_connection_retry_max_backoff` | `#[serde(default = "default_reverse_connection_retry_max_backoff", deserialize_with = "de_duration")]` |
| `tls_sni_override` | `#[serde(default, deserialize_with = "de_sni_override")]` |
| `tls_sni_disable` | `#[serde(default)]` |
| `tls_ech_enable` | `#[serde(default)]` |
| `tls_verify_certificate` | `#[serde(default)]` |
| `http_proxy` | `#[serde(default)]` |
| `http_proxy_login` | `#[serde(default)]` |
| `http_proxy_password` | `#[serde(default)]` |
| `http_upgrade_path_prefix` | `#[serde(default = "default_http_upgrade_path_prefix")]` |
| `http_upgrade_credentials` | `#[serde(default, deserialize_with = "de_http_credentials")]` |
| `websocket_ping_frequency` | `#[serde(default = "default_websocket_ping_frequency", deserialize_with = "de_duration_opt")]` |
| `websocket_mask_frame` | `#[serde(default)]` |
| `http_headers` | `#[serde(default, deserialize_with = "de_http_headers")]` |
| `http_headers_file` | `#[serde(default)]` |
| `remote_addr` | `#[serde(deserialize_with = "de_server_url")]` (NO default — required) |
| `tls_certificate` | `#[serde(default)]` |
| `tls_private_key` | `#[serde(default)]` |
| `dns_resolver` | `#[serde(default, deserialize_with = "de_url_vec")]` |
| `dns_resolver_prefer_ipv4` | `#[serde(default)]` |

Example (the `local_to_remote` field becomes):

```rust
    #[cfg_attr(feature = "clap", arg(short='L', long, value_name = "{tcp,udp,socks5,stdio,unix}://[BIND:]PORT:HOST:PORT", value_parser = parsers::parse_tunnel_arg, verbatim_doc_comment))]
    #[serde(default, deserialize_with = "de_tunnels")]
    pub local_to_remote: Vec<LocalToRemote>,
```

- [ ] **Step 6: Add the `config` field**

At the **end** of the `Client` struct (after `dns_resolver_prefer_ipv4`, before the closing `}` at line 248), add:

```rust
    /// Read all client settings from a TOML config file.
    /// When set, no other client option may be provided.
    #[cfg_attr(feature = "clap", arg(
        long,
        value_name = "FILE_PATH",
        verbatim_doc_comment,
        conflicts_with_all = ["local_to_remote", "remote_to_local", "socket_so_mark",
            "connection_min_idle", "connection_retry_max_backoff",
            "reverse_tunnel_connection_retry_max_backoff", "tls_sni_override",
            "tls_sni_disable", "tls_ech_enable", "tls_verify_certificate", "http_proxy",
            "http_proxy_login", "http_proxy_password", "http_upgrade_path_prefix",
            "http_upgrade_credentials", "websocket_ping_frequency", "websocket_mask_frame",
            "http_headers", "http_headers_file", "remote_addr", "tls_certificate",
            "tls_private_key", "dns_resolver", "dns_resolver_prefer_ipv4"]
    ))]
    #[serde(skip)]
    pub config: Option<PathBuf>,
```

- [ ] **Step 7: Make `remote_addr` not required when `--config` is present**

In `remote_addr`'s `#[cfg_attr(feature = "clap", arg(...))]` (around line 207), add `required_unless_present = "config"` to the `arg(...)` list. It becomes:

```rust
    #[cfg_attr(feature = "clap", arg(value_name = "ws[s]|http[s]://wstunnel.server.com[:port]", value_parser = parsers::parse_server_url, verbatim_doc_comment, required_unless_present = "config"))]
    #[serde(deserialize_with = "de_server_url")]
    pub remote_addr: Url,
```

- [ ] **Step 8: Add `Client::from_config_file`**

Immediately **after** the `Client` struct definition (after its closing `}` near line 248/249), add:

```rust
impl Client {
    /// Load a client configuration from a TOML file.
    pub fn from_config_file(path: &std::path::Path) -> anyhow::Result<Client> {
        use anyhow::Context;
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Cannot read client config file: {}", path.display()))?;
        toml::from_str::<Client>(&content)
            .with_context(|| format!("Cannot parse client config file: {}", path.display()))
    }
}
```

- [ ] **Step 9: Run the tests to verify they pass**

Run: `cargo test -p webtop --features clap config_file_tests`
Expected: PASS — all 7 tests green.

- [ ] **Step 10: Verify the whole lib still builds both ways**

Run: `cargo build -p webtop --no-default-features --features aws-lc-rs`
Expected: compiles (no unused-`parsers` warnings now — the `Deserialize` impl uses them).

Run: `cargo test -p webtop --features clap`
Expected: existing tests + new tests all pass.

- [ ] **Step 11: Commit**

```bash
git add wstunnel/src/config.rs
git commit -m "feat: deserialize client config from TOML reusing CLI parsers"
```

---

### Task 3: Wire `--config` into the CLI

Load the `Client` from the TOML file when `--config` is given, and do it **before** logging setup (so a `stdio://` tunnel defined in the file still routes logs to stderr).

**Files:**
- Modify: `wstunnel-cli/src/main.rs`

**Interfaces:**
- Consumes: `Client::from_config_file(&Path) -> anyhow::Result<Client>` (Task 2); `Client` is `Clone`.
- Produces: CLI behavior — `client --config <FILE>` loads from TOML; `--config` errors if combined with any other client arg; `client` without `--config` still requires a server URL.

- [ ] **Step 1: Write the failing CLI parse tests**

Append to the **end** of `wstunnel-cli/src/main.rs`:

```rust
#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn config_flag_parses_alone() {
        let res = Wstunnel::try_parse_from(["webtop", "client", "--config", "x.toml"]);
        assert!(res.is_ok(), "expected ok, got {res:?}");
    }

    #[test]
    fn config_flag_conflicts_with_tunnel() {
        let res = Wstunnel::try_parse_from([
            "webtop", "client", "--config", "x.toml", "-L", "tcp://1212:google.com:443",
        ]);
        assert!(res.is_err(), "expected conflict error, got {res:?}");
    }

    #[test]
    fn config_flag_conflicts_with_server_url() {
        let res = Wstunnel::try_parse_from([
            "webtop", "client", "--config", "x.toml", "wss://server.example.com:443",
        ]);
        assert!(res.is_err(), "expected conflict error, got {res:?}");
    }

    #[test]
    fn client_without_config_requires_server_url() {
        let res = Wstunnel::try_parse_from(["webtop", "client", "-L", "tcp://1212:google.com:443"]);
        assert!(res.is_err(), "expected missing-remote_addr error, got {res:?}");
    }
}
```

(`try_parse_from` comes from `clap::Parser`, already imported at the top of `main.rs`.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p webtop-cli cli_tests`
Expected: FAIL — `config_flag_parses_alone` fails (today `client` requires a server URL, and `--config` does not exist yet → all four likely error or the binary doesn't compile because `--config` is unknown until Task 2 is built; if it compiles, `parses_alone` fails).

- [ ] **Step 3: Resolve the client config before logging setup**

In `wstunnel-cli/src/main.rs`, replace the section from `let args = Wstunnel::parse();` down to the end of the logging `if let Commands::Client(args) = ... { ... } else { ... };` block (current lines 63–90) with:

```rust
    let args = Wstunnel::parse();

    // Resolve the effective client config up front: load from the TOML file when
    // --config is given. Done before logging setup because a stdio tunnel (which
    // may be defined in the file) requires logs to go to stderr.
    let client: Option<Client> = match &args.commands {
        Commands::Client(c) => Some(match &c.config {
            Some(path) => Client::from_config_file(path)?,
            None => (**c).clone(),
        }),
        Commands::Server(_) => None,
    };

    // Setup logging
    let mut env_filter = EnvFilter::builder().parse(&args.log_lvl).expect("Invalid log level");
    if !(args.log_lvl.contains("h2::") || args.log_lvl.contains("h2=")) {
        env_filter = env_filter.add_directive(Directive::from_str("h2::codec=off").expect("Invalid log directive"));
    }
    let logger = tracing_subscriber::fmt()
        .with_ansi(args.no_color.is_none())
        .with_target(false)
        .with_env_filter(env_filter);

    // stdio tunnel capture stdio, so need to log into stderr
    if let Some(client) = &client {
        if client
            .local_to_remote
            .iter()
            .filter(|x| matches!(x.local_protocol, LocalProtocol::Stdio { .. }))
            .count()
            > 0
        {
            logger.with_writer(io::stderr).init();
        } else {
            logger.init()
        }
    } else {
        logger.init();
    };
```

- [ ] **Step 4: Use the resolved client in the run match**

Replace the final `match args.commands { ... }` block (current lines 95–110) with:

```rust
    match args.commands {
        Commands::Client(_) => {
            run_client(client.expect("client config resolved above"), DefaultTokioExecutor::default())
                .await
                .unwrap_or_else(|err| {
                    panic!("Cannot start webtop client: {err:?}");
                });
        }
        Commands::Server(args) => {
            run_server(*args, DefaultTokioExecutor::default())
                .await
                .unwrap_or_else(|err| {
                    panic!("Cannot start webtop server: {err:?}");
                });
        }
    }
```

- [ ] **Step 5: Run the CLI tests to verify they pass**

Run: `cargo test -p webtop-cli cli_tests`
Expected: PASS — all four tests green.

- [ ] **Step 6: Verify the binary builds and the flag shows in help**

Run: `cargo run -p webtop-cli -- client --help`
Expected: help text lists `--config <FILE_PATH>`.

- [ ] **Step 7: Smoke-test loading a real file (error path is fine)**

Run:
```bash
printf 'remote_addr = "wss://does-not-exist.example:443"\nlocal_to_remote = ["tcp://12121:127.0.0.1:1"]\n' > /tmp/webtop-smoke.toml
cargo run -p webtop-cli -- client --config /tmp/webtop-smoke.toml
```
Expected: the client starts and attempts to connect (then fails to reach the bogus server) — confirming the file was parsed and used. Ctrl-C to stop. Then `rm /tmp/webtop-smoke.toml`.

- [ ] **Step 8: Commit**

```bash
git add wstunnel-cli/src/main.rs
git commit -m "feat: load client config from --config TOML file"
```

---

### Task 4: Document the client config file in the README

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: schema from Tasks 2–3.
- Produces: user-facing docs.

- [ ] **Step 1: Add a config-file section**

In `README.md`, add a new subsection under the client documentation (a good anchor is right after the client `--config`/options reference, near the existing client usage examples). Insert:

````markdown
### Client config file

Instead of passing client options on the command line, you can put them all in a
TOML file and run:

```bash
wstunnel client --config tunnels.toml
```

`--config` is **client-only** and cannot be combined with any other client
option. Keys match the long option names, and tunnels use the same URL syntax as
`-L`/`-R`. Only `remote_addr` is required; every other key falls back to the same
default as the command line.

```toml
# server URL (the only required key)
remote_addr = "wss://server.example.com:443"

# tunnels — same syntax as -L / -R, listed in arrays
local_to_remote = [
  "tcp://1212:google.com:443",
  "udp://1212:1.1.1.1:53?timeout_sec=10",
  "socks5://[::1]:1212",
]
remote_to_local = ["tcp://1213:google.com:443"]

# optional settings (omit any you don't need)
tls_verify_certificate = true
tls_sni_override = "example.com"
http_proxy = "user:pass@host:8080"
connection_min_idle = 5
connection_retry_max_backoff = "5m"
websocket_ping_frequency = "30s"
http_headers = ["X-Foo: bar"]
http_upgrade_credentials = "user:pass"
dns_resolver = ["dns://1.1.1.1"]
http_upgrade_path_prefix = "v1"
```

Unknown keys are rejected, so typos fail fast rather than being silently ignored.
````

- [ ] **Step 2: Verify the markdown renders sanely**

Run: `grep -n "Client config file" README.md`
Expected: one match at the new section.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: document client TOML config file"
```

---

## Self-Review

**1. Spec coverage:**
- Config-only mode + `--config` flag → Task 2 (`config` field, conflicts) + Task 3 (loading). ✓
- Tunnels as URL strings reusing parsers → Task 2 `de_tunnels`/`de_reverse_tunnels`. ✓
- Keys map 1:1 to field names → Task 2 Step 5. ✓
- `deny_unknown_fields` → Task 2 Step 4 + test `unknown_key_is_rejected`. ✓
- `remote_addr` required, others default → Task 2 (no default on `remote_addr`, `required_unless_present`) + tests. ✓
- Deserialization mechanics (shared parsers) → Task 2 Step 3. ✓
- Errors as `anyhow::Error` before start → Task 2 `from_config_file` + Task 3 `?`. ✓
- `toml` dependency → Task 1. ✓
- Tests (round-trip, all-fields, failure cases) → Task 2 Step 1. ✓
- README section → Task 4. ✓
- Stdio-tunnel-in-file logging edge case → Task 3 (resolve before logging). ✓ (caught during planning)

**2. Placeholder scan:** No TBD/TODO/"handle errors"/"similar to" — all steps contain concrete code. ✓

**3. Type consistency:** `Client::from_config_file(&std::path::Path) -> anyhow::Result<Client>` used identically in Task 2 (definition) and Task 3 (call). `config: Option<PathBuf>` defined in Task 2, read in Task 3. Helper names (`de_tunnels`, `de_duration`, `default_websocket_ping_frequency`, …) referenced in Step 5 match their definitions in Step 3. ✓
