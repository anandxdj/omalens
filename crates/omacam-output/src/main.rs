mod cli;
mod config;
mod decoder;
mod mux;
mod pipeline;
mod preview;
mod service;

#[cfg(test)]
mod tests;

fn main() -> std::process::ExitCode {
    cli::run()
}
