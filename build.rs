// Copies runtime/pdfium.dll next to the build output. The app loads PDFium
// only from its own folder, so development runs and tests need it there.
use std::path::{Path, PathBuf};

fn main() {
    let source = Path::new("runtime/pdfium.dll");
    // A missing file reruns this script on every build, until the file exists.
    println!("cargo:rerun-if-changed={}", source.display());
    // OUT_DIR is target/<profile>/build/<package>-<hash>/out.
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let Some(profile) = out.ancestors().nth(3) else {
        return;
    };
    if source.is_file() {
        std::fs::copy(source, profile.join("pdfium.dll"))
            .expect("Could not copy runtime/pdfium.dll next to the build output");
    }
}
