use log::LevelFilter;
use structopt::{clap, StructOpt};

#[derive(Debug, StructOpt)]
#[structopt(name = "svls")]
#[structopt(long_version(option_env!("LONG_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))))]
#[structopt(setting(clap::AppSettings::ColoredHelp))]
pub struct Opt {
    #[structopt(
        long = "log-level", 
        possible_values = &["off", "trace", "debug", "info", "warn", "error"], 
        default_value = "off", 
        help = "The level of log printing"
    )]
    pub log_level: LevelFilter,

    #[structopt(
        long = "log-file",
        default_value = "svls.log",
        help = "The file to print log information to"
    )]
    pub log_file: String,
}
