#![windows_subsystem = "windows"]

mod background;
mod imaging;
mod model;
mod ocr;
mod pdf;
mod printing;
mod shell;

fn main() {
    if let Err(error) = shell::run() {
        eprintln!("Preview could not start: {error}");
        std::process::exit(1);
    }
}
