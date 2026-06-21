use clap::Parser;
use std::io;
use std::path::PathBuf;
use std::str::FromStr;
use tracing::warn;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::Directive;
use webtop::LocalProtocol;
use webtop::config::{Client, Server};
use webtop::executor::DefaultTokioExecutor;
use webtop::{run_client, run_server};

#[cfg(feature = "jemalloc")]
use tikv_jemallocator::Jemalloc;

#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

/// Default client config file looked up in the current directory when neither `--config`
/// nor a server URL is given on the command line.
const DEFAULT_CLIENT_CONFIG_FILE: &str = ".webtop.toml";

/// Decide where the effective client config comes from:
///   1. `--config <FILE>` explicitly given  -> load that file (config-only).
///   2. no server URL on the CLI            -> load `default_config_path` if present,
///                                             else error (nothing to connect to).
///   3. a server URL was given on the CLI    -> use the CLI args as-is.
fn resolve_client_config(c: &Client, default_config_path: Option<PathBuf>) -> anyhow::Result<Client> {
    if let Some(path) = &c.config {
        return Client::from_config_file(path);
    }
    if c.remote_addr_is_placeholder() {
        if let Some(path) = default_config_path {
            return Client::from_config_file(&path);
        }
        anyhow::bail!(
            "no server URL provided. Pass it on the command line, use --config <FILE>, or create {DEFAULT_CLIENT_CONFIG_FILE} in the current directory"
        );
    }
    Ok(c.clone())
}

/// webtop the new web top tool
#[derive(clap::Parser, Debug)]
#[command(name = "webtop", author, version, about, verbatim_doc_comment, long_about = None, disable_help_subcommand = true, args_conflicts_with_subcommands = true)]
pub struct Wstunnel {
    #[command(subcommand)]
    commands: Option<Commands>,

    /// Client options used when no subcommand is given (`client` is the default).
    #[command(flatten)]
    client: Box<Client>,

    /// Disable color output in logs
    #[arg(long, global = true, verbatim_doc_comment, env = "NO_COLOR")]
    no_color: Option<String>,

    /// *WARNING* The flag does nothing, you need to set the env variable *WARNING*
    /// Control the number of threads that will be used.
    /// By default, it is equal the number of cpus
    #[arg(
        long,
        global = true,
        value_name = "INT",
        verbatim_doc_comment,
        env = "TOKIO_WORKER_THREADS"
    )]
    nb_worker_threads: Option<u32>,

    /// Control the log verbosity. i.e: TRACE, DEBUG, INFO, WARN, ERROR, OFF
    /// for more details: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#example-syntax
    #[arg(
        long,
        global = true,
        value_name = "LOG_LEVEL",
        verbatim_doc_comment,
        env = "RUST_LOG",
        default_value = "WARN"
    )]
    log_lvl: String,
}

#[derive(clap::Subcommand, Debug)]
pub enum Commands {
    Client(Box<Client>),
    Server(Box<Server>),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Wstunnel::parse();

    // Resolve the effective client config up front: load from the TOML file when
    // --config is given. Done before logging setup because a stdio tunnel (which
    // may be defined in the file) requires logs to go to stderr.
    let client: Option<Client> = match &args.commands {
        // No subcommand given: behave as `client` using the top-level flattened args.
        None => {
            let default_config = Some(PathBuf::from(DEFAULT_CLIENT_CONFIG_FILE)).filter(|p| p.exists());
            Some(resolve_client_config(&args.client, default_config)?)
        }
        Some(Commands::Client(c)) => {
            let default_config = Some(PathBuf::from(DEFAULT_CLIENT_CONFIG_FILE)).filter(|p| p.exists());
            Some(resolve_client_config(c, default_config)?)
        }
        Some(Commands::Server(_)) => None,
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
    if let Err(err) = fdlimit::raise_fd_limit() {
        warn!("Failed to set soft filelimit to hard file limit: {}", err)
    }

    match args.commands {
        None | Some(Commands::Client(_)) => {
            run_client(client.expect("client config resolved above"), DefaultTokioExecutor::default())
                .await
                .unwrap_or_else(|err| {
                    panic!("Cannot start webtop client: {err:?}");
                });
        }
        Some(Commands::Server(args)) => {
            run_server(*args, DefaultTokioExecutor::default())
                .await
                .unwrap_or_else(|err| {
                    panic!("Cannot start webtop server: {err:?}");
                });
        }
    }

    Ok(())
}

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

    fn parse_client(argv: &[&str]) -> webtop::config::Client {
        let w = Wstunnel::try_parse_from(argv).unwrap_or_else(|e| panic!("parse failed: {e}"));
        match w.commands {
            Some(Commands::Client(c)) => *c,
            Some(Commands::Server(_)) => panic!("expected client subcommand"),
            // No subcommand: the client args are parsed into the top-level flattened struct.
            None => *w.client,
        }
    }

    #[test]
    fn no_subcommand_defaults_to_client() {
        // A bare server URL with no subcommand is parsed as a client connection.
        let c = parse_client(&["webtop", "wss://default-sub.example:9090"]);
        assert_eq!(c.remote_addr.as_str(), "wss://default-sub.example:9090/");
    }

    #[test]
    fn no_subcommand_accepts_client_flags() {
        // Client flags work without the `client` subcommand.
        let c = parse_client(&["webtop", "-L", "tcp://1212:google.com:443", "wss://srv.example:9090"]);
        assert_eq!(c.local_to_remote.len(), 1);
        assert_eq!(c.remote_addr.as_str(), "wss://srv.example:9090/");
    }

    #[test]
    fn bare_invocation_uses_placeholder() {
        // No subcommand and no URL: parses to the placeholder, resolved later against the config file.
        let c = parse_client(&["webtop"]);
        assert!(c.remote_addr_is_placeholder());
    }

    #[test]
    fn bare_client_without_default_file_errors() {
        let c = parse_client(&["webtop", "client"]);
        let res = resolve_client_config(&c, None);
        assert!(res.is_err(), "expected error when no URL and no default file, got {res:?}");
    }

    #[test]
    fn bare_client_loads_default_file_when_present() {
        let path = std::env::temp_dir().join("webtop_default_file_test.toml");
        std::fs::write(&path, "remote_addr = \"ws://from-file.example:8080\"\n").unwrap();
        let c = parse_client(&["webtop", "client"]);
        let resolved = resolve_client_config(&c, Some(path.clone())).expect("should load default file");
        assert_eq!(resolved.remote_addr.as_str(), "ws://from-file.example:8080/");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn cli_server_url_ignores_default_file() {
        let path = std::env::temp_dir().join("webtop_ignored_file_test.toml");
        std::fs::write(&path, "remote_addr = \"ws://from-file.example:8080\"\n").unwrap();
        let c = parse_client(&["webtop", "client", "ws://from-cli.example:9090"]);
        let resolved = resolve_client_config(&c, Some(path.clone())).expect("ok");
        assert_eq!(resolved.remote_addr.as_str(), "ws://from-cli.example:9090/");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn explicit_config_flag_wins_over_default_file() {
        let explicit = std::env::temp_dir().join("webtop_explicit_cfg_test.toml");
        let other = std::env::temp_dir().join("webtop_other_default_test.toml");
        std::fs::write(&explicit, "remote_addr = \"ws://explicit.example:7070\"\n").unwrap();
        std::fs::write(&other, "remote_addr = \"ws://default.example:1010\"\n").unwrap();
        let c = parse_client(&["webtop", "client", "--config", explicit.to_str().unwrap()]);
        let resolved = resolve_client_config(&c, Some(other.clone())).expect("ok");
        assert_eq!(resolved.remote_addr.as_str(), "ws://explicit.example:7070/");
        let _ = std::fs::remove_file(&explicit);
        let _ = std::fs::remove_file(&other);
    }
}
