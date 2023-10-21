use clap::{Parser, Subcommand};
use stalagmite::{generate, initialize_project, run_dev_server};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init,
    Gen,
    DevServer,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Init => initialize_project().unwrap(),
        Commands::Gen => generate(),
        Commands::DevServer => run_dev_server().await,
    }
}
