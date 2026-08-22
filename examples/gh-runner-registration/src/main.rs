use std::path::PathBuf;

use clap::Parser;
use gh_actions_listener::{Error, Registration};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    url: String,
    #[arg(long)]
    token: String,
    #[arg(long, default_value = "canopy")]
    name: String,
    #[arg(long, value_delimiter = ',')]
    labels: Vec<String>,
    #[arg(long, default_value = "credentials.json")]
    credentials: PathBuf,
}

fn main() -> Result<(), Error> {
    let args = Args::parse();
    let registration = Registration {
        url: args.url,
        token: args.token,
        name: args.name,
        labels: args.labels,
    };

    gh_actions_listener::register(&registration)?.write(&args.credentials)?;
    println!("registered; wrote {}", args.credentials.display());
    Ok(())
}
