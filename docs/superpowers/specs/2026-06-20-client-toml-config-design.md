# Client TOML config file — design

**Date:** 2026-06-20
**Status:** Approved (pending spec review)
**Scope:** `client` mode only. `server` mode is untouched.

## Goal

Let users specify all wstunnel **client** settings — including multiple
tunnels — from a TOML config file instead of command-line arguments.

## Decisions

- **Config-only mode.** When `--config <FILE>` is given, every client setting
  comes from the file. The flag conflicts with all other client options; passing
  both is an error.
- **Tunnels are URL strings**, exactly the strings the CLI already accepts
  (e.g. `tcp://1212:google.com:443`). No new tunnel grammar. This reuses the
  existing parsers and keeps one source of truth for formats.
- **Keys map 1:1 to `Client` struct field names** (snake_case), which already
  match the CLI long flags.

## CLI integration & control flow

Add a `--config <FILE>` flag (long-only — `-c` is taken by
`connection_min_idle`) to the `client` subcommand only.

- `wstunnel client --config tunnels.toml` → load the whole `Client` from TOML.
- `wstunnel client <url> -L ...` → unchanged.
- `--config` uses clap `conflicts_with_all` against the other client args, so
  combining it with any other client option is a hard error:
  *"--config cannot be combined with other client options"*.
- The positional `remote_addr` becomes `required_unless_present = "config"`, so
  clap does not demand a server URL when a config file is supplied.

Flow in `wstunnel-cli/src/main.rs`: after clap parsing, if `client.config` is
`Some(path)`, call `Client::from_config_file(path)` to build the real `Client`,
then pass it to `run_client`. The `config` field is `#[serde(skip)]` and never
appears in the file.

## TOML schema

```toml
# server URL (was the positional arg) — the ONLY required key
remote_addr = "wss://server.example.com:443"

# the multiple tunnels
local_to_remote = [
  "tcp://1212:google.com:443",
  "udp://1212:1.1.1.1:53?timeout_sec=10",
  "socks5://[::1]:1212",
]
remote_to_local = ["tcp://1212:google.com:443"]

# optional client settings (all have defaults; omit any you don't need)
tls_verify_certificate = true
tls_sni_override = "example.com"
http_proxy = "user:pass@host:8080"
connection_min_idle = 5
connection_retry_max_backoff = "5m"          # same duration syntax as CLI
websocket_ping_frequency = "30s"
http_headers = ["X-Foo: bar", "X-Baz: qux"]
http_upgrade_credentials = "user:pass"
dns_resolver = ["dns://1.1.1.1"]
http_upgrade_path_prefix = "v1"
```

- `remote_addr` is the only required key. Every other field uses
  `#[serde(default)]` backed by a `Default` impl matching the CLI defaults.
- `deny_unknown_fields`: a typo'd or unknown key is a hard error, not silently
  ignored.

## Deserialization mechanics

`Client` gains `#[derive(Deserialize)]` (gated behind the `clap` feature, since
it reuses the `parsers` module that lives behind that feature). Each field whose
CLI form uses a custom `value_parser` gets a matching
`#[serde(deserialize_with = ...)]` that takes the **string** and calls the
*same* `parsers::*` function. There is exactly one parsing implementation shared
by both the CLI and the config file.

Field-by-field:

- `local_to_remote` / `remote_to_local`: `Vec<String>` → `parse_tunnel_arg` /
  `parse_reverse_tunnel_arg` per element.
- `remote_addr`: `String` → `parse_server_url`.
- `connection_retry_max_backoff`,
  `reverse_tunnel_connection_retry_max_backoff`, `websocket_ping_frequency`:
  `String` ("5m") → `parse_duration_sec`.
- `http_headers`: `Vec<String>` → `parse_http_headers`.
- `tls_sni_override`: `String` → `parse_sni_override`.
- `http_upgrade_credentials`: `String` → `parse_http_credentials`.
- `http_proxy`, `http_proxy_login`, `http_proxy_password`,
  `http_upgrade_path_prefix`, paths (`tls_certificate`, `tls_private_key`,
  `http_headers_file`), `dns_resolver` (`Vec<Url>` via string), and the plain
  bool/int fields use straightforward serde (string/number → type), with custom
  helpers only where a type lacks a direct serde impl.

`Client::from_config_file(path)` reads the file, parses with `toml`, and returns
`anyhow::Result<Client>`.

## Errors

All surfaced as a readable message before the client starts, returned as
`anyhow::Error` from `from_config_file` (rather than a raw panic):

- file not found / unreadable
- invalid TOML syntax
- unknown key (`deny_unknown_fields`)
- per-field parse error (bad tunnel string, bad duration, missing `remote_addr`)

## Dependencies

- Add `toml` to the `webtop` crate (`wstunnel/Cargo.toml`). `serde` is already a
  dependency.

## Testing

Unit tests in `wstunnel/src/config.rs` (no network needed):

- round-trip a representative config with multiple tunnels and most optional
  fields omitted → assert defaults applied
- a config exercising all fields → assert each parsed value
- failure cases: unknown key, malformed tunnel string, missing `remote_addr`

## Docs

Add a "Config file (client)" section to `README.md` with the schema example
above.

## Out of scope

- Server-mode config file (server already has `--restrict-config`).
- Merging config file with CLI overrides (explicitly rejected — config-only).
- Hot-reload of the client config file.
