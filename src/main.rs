use clap::{Parser, Subcommand};
use stalagmite::{generate, project, run_dev_server};

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
    Add {
        #[command(subcommand)]
        add_command: AddCommand,
    },
    Gen,
    DevServer,
}

#[derive(Subcommand)]
enum AddCommand {
    Page { path: String, title: String },
    Layout { path: String },
    Rules { path: String },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Init => project::initialize().unwrap(),
        // TODO (maybe): these path arguments are pretty UNIX centric...
        Commands::Add { add_command } => match add_command {
            AddCommand::Page { path, title } => {
                project::add_page(path, title).unwrap();
                println!("Added page at {} with title {}", path, title);
            }
            AddCommand::Layout { .. } => unimplemented!(),
            AddCommand::Rules { path } => {
                project::add_rule_set(path).unwrap();
                println!("Added rule set at {}", path);
            }
        },
        Commands::Gen => generate(),
        // TODO devserver should be an optional feature
        Commands::DevServer => run_dev_server().await,
    }
}
