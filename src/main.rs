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
mod ui;

fn main() {
    if let Err(error) = ui::run() {
        eprintln!("Preview could not start: {error}");
        std::process::exit(1);
    }
}
