use backend::Backend;
use log::debug;
use opt::Opt;
use simplelog::{Config, LevelFilter, WriteLogger};
use std::error::Error;
use std::fs::File;
use structopt::StructOpt;
use tower_lsp::{LspService, Server};

mod backend;
mod config;
mod opt;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let opt = Opt::from_args();

    if opt.log_level != LevelFilter::Off {
        WriteLogger::init(
            opt.log_level,
            Config::default(),
            File::create(opt.log_file)?,
        )?;
    }

    debug!("start");

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, messages) = LspService::new(Backend::new);
    Server::new(stdin, stdout)
        .interleave(messages)
        .serve(service)
        .await;

    Ok(())
}
