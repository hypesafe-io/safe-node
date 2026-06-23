use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "safe-node", version, about = "Private HypeSafe signing node")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the daemon loop.
    Run(RunArgs),
    /// Process one polling cycle and exit.
    Once(OnceArgs),
    /// Show the terminal UI backed by the local debug HTTP endpoint.
    Tui(TuiArgs),
    /// Manage encrypted signer keystores.
    Keystore(KeystoreArgs),
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// JSON5 config file path. Defaults to config/node.json.
    #[arg(long)]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct OnceArgs {
    /// JSON5 config file path. Defaults to config/node.json.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Evaluate without signing, submitting signatures, or submitting to HL.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct TuiArgs {
    /// Debug HTTP base URL.
    #[arg(long, default_value = "http://127.0.0.1:9909")]
    pub url: String,
    /// Number of recent transactions to display.
    #[arg(long, default_value_t = 20)]
    pub limit: u32,
    /// Refresh interval in seconds.
    #[arg(long, default_value_t = 2)]
    pub refresh: u64,
}

#[derive(Debug, Args)]
pub struct KeystoreArgs {
    #[command(subcommand)]
    pub command: KeystoreCommand,
}

#[derive(Debug, Subcommand)]
pub enum KeystoreCommand {
    /// Generate a new signer key and write it to an encrypted JSON keystore.
    Generate(KeystoreGenerateArgs),
    /// Import an existing private key into an encrypted JSON keystore.
    Import(KeystoreImportArgs),
}

#[derive(Debug, Args)]
pub struct KeystoreGenerateArgs {
    /// Output keystore path.
    #[arg(long)]
    pub out: PathBuf,
    /// Overwrite an existing output file.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct KeystoreImportArgs {
    /// Output keystore path.
    #[arg(long)]
    pub out: PathBuf,
    /// Overwrite an existing output file.
    #[arg(long)]
    pub force: bool,
}

#[must_use]
pub fn default_config_path() -> PathBuf {
    PathBuf::from("config/node.json")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::Parser;

    use super::{Cli, Command, KeystoreCommand};

    #[test]
    fn parses_keystore_generate() {
        let cli = Cli::parse_from([
            "safe-node",
            "keystore",
            "generate",
            "--out",
            "config/signer.json",
        ]);

        match cli.command {
            Command::Keystore(args) => match args.command {
                KeystoreCommand::Generate(generate) => {
                    assert_eq!(generate.out, PathBuf::from("config/signer.json"));
                    assert!(!generate.force);
                }
                KeystoreCommand::Import(_) => panic!("expected generate command"),
            },
            Command::Run(_) | Command::Once(_) | Command::Tui(_) => {
                panic!("expected keystore command");
            }
        }
    }
}
