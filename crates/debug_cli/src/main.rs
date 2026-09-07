//! Thin alias binary for source builds (`cargo run -p debug_cli`).
//! Release archives ship only `spec_chum` — use `spec_chum --serve` or
//! `spec_chum debug …` instead ([#231](https://github.com/mward-sudo/spec_chum/issues/231)).

fn main() {
    if let Err(e) = debug_cli::run() {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
