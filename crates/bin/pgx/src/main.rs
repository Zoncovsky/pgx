//! `pgx` — a local, disposable, PostgreSQL-compatible runtime.
mod cli;
mod cmd;
mod conf;
mod config;
mod errors;
mod fsdetect;
mod fsutil;
mod layout;
mod names;
mod out;
mod pgwire;
mod process;

use cli::{Command, ExitCode};
use errors::{PgxError, Result};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (global, command) = match cli::parse(&args) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("pgx: {e}");
            std::process::exit(e.code as i32);
        }
    };
    let code = match run(global, command) {
        Ok(code) => code,
        Err(e) => {
            if global.json {
                println!("{}", e.to_json());
            } else {
                eprintln!("pgx: {e}");
            }
            e.code
        }
    };
    std::process::exit(code as i32);
}

fn run(global: cli::Global, command: Command) -> Result<ExitCode> {
    let out = out::Out::new(global.json);
    match command {
        Command::Help => {
            print!("{}", cli::help_text());
            Ok(ExitCode::Ok)
        }
        Command::Version => cmd::version::run(&out),
        Command::Init { port, durability, force } => cmd::init::run(&out, port, durability, force),
        Command::Doctor => cmd::doctor::run(&out),
        other => {
            let home = layout::Home::resolve();
            if !home.is_initialized() {
                return Err(PgxError::new(ExitCode::NotReady, format!("PGX is not initialised at {}", home.root.display())).hint("run `pgx init` first"));
            }
            match other {
                Command::Start => cmd::start::run(&out),
                Command::Stop { force } => cmd::stop::run(&out, force),
                Command::Restart => cmd::start::restart(&out),
                Command::Status => cmd::status::run(&out),
                Command::Create { name } => cmd::db::create(&out, &name),
                Command::Drop { name, force } => cmd::db::drop(&out, &name, force),
                Command::Clone { source, target } => cmd::db::clone(&out, &source, &target),
                Command::Ls => cmd::db::ls(&out),
                Command::Url { name, socket } => cmd::db::url(&out, &name, socket),
                Command::Help | Command::Version | Command::Init { .. } | Command::Doctor => unreachable!(),
            }
        }
    }
}
