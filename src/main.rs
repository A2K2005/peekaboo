#![windows_subsystem = "windows"]

mod background;
mod imaging;
// The integration tests cover the whole module; the app calls only part of it.
#[allow(dead_code)]
mod integration;
mod model;
mod ocr;
mod pdf;
mod printing;
mod resident;
mod selection;
mod ui;

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = match args.first().and_then(|arg| arg.to_str()) {
        Some(integration::RESIDENT_FLAG) => resident::run(),
        Some(flag) if flag == integration::Action::Peek.flag() && resident::forward(&args) => Ok(()),
        _ => ui::run(),
    };
    if let Err(error) = result {
        eprintln!("Preview could not start: {error}");
        std::process::exit(1);
    }
}
