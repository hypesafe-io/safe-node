use clap::Parser;
use safe_node::cli::{default_config_path, Cli, Command, KeystoreCommand};
use safe_node::observe::tui;
use safe_node::{app, signing, Result};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();

    match cli.command {
        Command::Run(args) => {
            let config = args.config.unwrap_or_else(default_config_path);
            app::run(config).await
        }
        Command::Once(args) => {
            let config = args.config.unwrap_or_else(default_config_path);
            app::run_once(config, args.dry_run).await
        }
        Command::Tui(args) => tui::run(args.url, args.limit, args.refresh).await,
        Command::Keystore(args) => match args.command {
            KeystoreCommand::Generate(generate) => {
                signing::generate_keystore(&generate.out, generate.force)
            }
            KeystoreCommand::Import(import) => signing::import_keystore(&import.out, import.force),
        },
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,safe_node=debug"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
}
