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
    /// Initialize a new stalagmite site.
    Init,
    /// Add a new page, layout, or rule set to the project.
    Add {
        #[command(subcommand)]
        add_command: AddCommand,
    },
    /// Generate a static site from the current project.
    Gen,
    /// Run a local development server.
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
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    match &cli.command {
        Commands::Init => match project::initialize() {
            Ok(_) => println!("Initialized new stalagmite project"),
            Err(e) => {
                println!("Error initializing project: {}", e);
                // TODO should always return with exitcode 1 on error
                std::process::exit(1);
            }
        },
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