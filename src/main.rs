#![windows_subsystem = "windows"]

mod background;
mod imaging;
// Only argument parsing is wired into the shell.
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
