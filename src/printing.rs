use crate::{
    imaging,
    model::{frame_bytes, Frame, ImageEdit, PdfEdit, MAX_FRAME_BYTES},
    pdf::PdfEngine,
};
use std::{
    cell::RefCell,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{GlobalFree, HWND},
        Graphics::Gdi::*,
        Storage::Xps::{AbortDoc, EndDoc, EndPage, SetAbortProc, StartDocW, StartPage, DOCINFOW},
        UI::Controls::Dialogs::*,
    },
};

pub struct PrintJob {
    dc: HDC,
    first: u32,
    last: u32,
    cancel: Arc<AtomicBool>,
    started: bool,
}

// GDI handles may move between threads when exclusively owned. The UI never uses this DC after choose.
// https://learn.microsoft.com/windows/win32/procthread/multiple-threads-and-gdi-objects
unsafe impl Send for PrintJob {}

impl PrintJob {
    pub fn cancellation(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }
    fn check_cancelled(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Acquire) {
            Err("Printing was cancelled.".into())
        } else {
            Ok(())
        }
    }
}

impl Drop for PrintJob {
    fn drop(&mut self) {
        unsafe {
            if self.started {
                AbortDoc(self.dc);
            }
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Open only on the window thread. This does not submit a print job.
pub fn choose(owner: HWND, page_count: u32) -> Result<Option<PrintJob>, String> {
    if page_count == 0 || page_count > u16::MAX as u32 {
        return Err("The print dialog supports 1 to 65,535 pages.".into());
    }
    let mut dialog = PRINTDLGW {
        lStructSize: std::mem::size_of::<PRINTDLGW>() as u32,
        hwndOwner: owner,
        Flags: PD_RETURNDC | PD_NOSELECTION | PD_USEDEVMODECOPIESANDCOLLATE | PD_HIDEPRINTTOFILE,
        nFromPage: 1,
        nToPage: page_count as u16,
        nMinPage: 1,
        nMaxPage: page_count as u16,
        nCopies: 1,
        ..Default::default()
    };
    unsafe {
        let accepted = PrintDlgW(&mut dialog).as_bool();
        let error = if accepted {
            COMMON_DLG_ERRORS(0)
        } else {
            CommDlgExtendedError()
        };
        // The returned DC has its own DEVMODE; dialog allocation ownership stays here.
        if !dialog.hDevMode.is_invalid() {
            let _ = GlobalFree(Some(dialog.hDevMode));
        }
        if !dialog.hDevNames.is_invalid() {
            let _ = GlobalFree(Some(dialog.hDevNames));
        }
        if !accepted {
            if !dialog.hDC.is_invalid() {
                let _ = DeleteDC(dialog.hDC);
            }
            return if error.0 == 0 {
                Ok(None)
            } else {
                Err(format!(
                    "Windows could not open the print dialog ({}).",
                    error.0
                ))
            };
        }
        if dialog.hDC.is_invalid() {
            return Err("Windows did not return a printer.".into());
        }
        let range = if dialog.Flags.contains(PD_PAGENUMS) {
            checked_range(dialog.nFromPage as u32, dialog.nToPage as u32, page_count)
        } else {
            Ok((0, page_count - 1))
        };
        match range {
            Ok((first, last)) => Ok(Some(PrintJob {
                dc: dialog.hDC,
                first,
                last,
                cancel: Arc::new(AtomicBool::new(false)),
                started: false,
            })),
            Err(error) => {
                let _ = DeleteDC(dialog.hDC);
                Err(error)
            }
        }
    }
}

thread_local! { static CANCEL: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) }; }
unsafe extern "system" fn abort_print(_: HDC, _: i32) -> windows::core::BOOL {
    CANCEL
        .with(|flag| {
            flag.try_borrow()
                .map(|f| !f.as_ref().is_some_and(|f| f.load(Ordering::Acquire)))
                .unwrap_or(false)
        })
        .into()
}
struct CancelScope;
impl Drop for CancelScope {
    fn drop(&mut self) {
        CANCEL.with(|f| *f.borrow_mut() = None);
    }
}

/// A print job in progress. `page` prints one page per call, so the
/// document worker can render pages for the window between them. Run on
/// the existing decoder worker. No PDFium calls run outside its engine owner.
pub struct Printing {
    job: PrintJob,
    next: u32,
    is_pdf: bool,
    /// Left and top margins, then the printable width and height, in printer pixels.
    area: (i32, i32, i32, i32),
    dpi: (u32, u32),
    render: (u32, u32),
    _cancel: CancelScope,
}

/// Checks the printer and starts the document.
pub fn start(mut job: PrintJob, path: &Path, is_pdf: bool) -> Result<Printing, String> {
    job.check_cancelled()?;
    if !is_pdf && (job.first != 0 || job.last != 0) {
        return Err("An image has only one printable page.".into());
    }
    let (paper_width, paper_height, dpi_x, dpi_y) = unsafe {
        (
            GetDeviceCaps(Some(job.dc), HORZRES),
            GetDeviceCaps(Some(job.dc), VERTRES),
            GetDeviceCaps(Some(job.dc), LOGPIXELSX),
            GetDeviceCaps(Some(job.dc), LOGPIXELSY),
        )
    };
    if paper_width <= 0 || paper_height <= 0 || dpi_x <= 0 || dpi_y <= 0 {
        return Err("The printer reported an invalid page size.".into());
    }
    let margin_x = dpi_x / 4;
    let margin_y = dpi_y / 4;
    let width = paper_width.saturating_sub(2 * margin_x);
    let height = paper_height.saturating_sub(2 * margin_y);
    if width <= 0 || height <= 0 {
        return Err("The printable page is too small.".into());
    }
    let render = render_bounds(width as u32, height as u32, dpi_x as u32, dpi_y as u32)?;
    let name: Vec<u16> = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    CANCEL.with(|flag| *flag.borrow_mut() = Some(Arc::clone(&job.cancel)));
    let cancel = CancelScope;
    unsafe {
        if SetAbortProc(job.dc, Some(abort_print)) <= 0 {
            return Err("The printer could not set up cancellation.".into());
        }
        let info = DOCINFOW {
            cbSize: std::mem::size_of::<DOCINFOW>() as i32,
            lpszDocName: PCWSTR(name.as_ptr()),
            ..Default::default()
        };
        if StartDocW(job.dc, &info) <= 0 {
            return Err("The printer could not start this document.".into());
        }
        job.started = true;
    }
    Ok(Printing {
        next: job.first,
        job,
        is_pdf,
        area: (margin_x, margin_y, width, height),
        dpi: (dpi_x as u32, dpi_y as u32),
        render,
        _cancel: cancel,
    })
}

impl Printing {
    /// Prints the next page. Returns true while pages remain.
    pub fn page(
        &mut self,
        engine: &mut PdfEngine,
        path: &Path,
        pdf_edits: &[PdfEdit],
        image_edits: &[ImageEdit],
    ) -> Result<bool, String> {
        let job = &self.job;
        let (margin_x, margin_y, width, height) = self.area;
        let (render_width, render_height) = self.render;
        job.check_cancelled()?;
        let mut frame = if self.is_pdf {
            engine.render_for_print(path, self.next, render_width, render_height, pdf_edits)?
        } else {
            imaging::decode_edited(path, render_width, render_height, image_edits)?
        };
        if frame_bytes(frame.width, frame.height)? != frame.pixels.len() {
            return Err("Cannot print an invalid image buffer.".into());
        }
        composite_white(&mut frame);
        job.check_cancelled()?;
        // Printer pixels can have unequal horizontal and vertical density.
        let (dest_width, dest_height) = print_size(
            frame.width,
            frame.height,
            width as u32,
            height as u32,
            self.dpi.0,
            self.dpi.1,
        )?;
        let x = margin_x + (width - dest_width as i32) / 2;
        let y = margin_y + (height - dest_height as i32) / 2;
        let bitmap = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: frame.width as i32,
                biHeight: -(frame.height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        unsafe {
            if StartPage(job.dc) <= 0 {
                return Err("The printer could not start the next page.".into());
            }
            let rows = StretchDIBits(
                job.dc,
                x,
                y,
                dest_width as i32,
                dest_height as i32,
                0,
                0,
                frame.width as i32,
                frame.height as i32,
                Some(frame.pixels.as_ptr().cast()),
                &bitmap,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
            if rows == 0 || rows == -1 {
                return Err("The printer could not draw this page.".into());
            }
            job.check_cancelled()?;
            if EndPage(job.dc) <= 0 {
                return Err("The printer could not finish this page.".into());
            }
        }
        self.next += 1;
        Ok(self.next <= self.job.last)
    }

    /// Ends the document. Dropping an unfinished job aborts it.
    pub fn finish(mut self) -> Result<(), String> {
        self.job.check_cancelled()?;
        if unsafe { EndDoc(self.job.dc) } <= 0 {
            return Err("The printer could not finish the document.".into());
        }
        self.job.started = false;
        Ok(())
    }
}

fn checked_range(first: u32, last: u32, count: u32) -> Result<(u32, u32), String> {
    if first == 0 || first > last || last > count {
        return Err("Select a valid print page range.".into());
    }
    Ok((first - 1, last - 1))
}

fn render_bounds(width: u32, height: u32, dpi_x: u32, dpi_y: u32) -> Result<(u32, u32), String> {
    if width == 0 || height == 0 || dpi_x == 0 || dpi_y == 0 {
        return Err("Invalid printer dimensions.".into());
    }
    // Rasterize at up to 300 DPI with square source pixels and a bounded page allocation.
    let mut w = width as f64 * 300.0 / dpi_x as f64;
    let mut h = height as f64 * 300.0 / dpi_y as f64;
    let limit = (MAX_FRAME_BYTES as f64 / (4.0 * w * h)).sqrt().min(1.0);
    w = (w * limit).floor().max(1.0);
    h = (h * limit).floor().max(1.0);
    frame_bytes(w as u32, h as u32)?;
    Ok((w as u32, h as u32))
}

fn print_size(
    w: u32,
    h: u32,
    max_w: u32,
    max_h: u32,
    dpi_x: u32,
    dpi_y: u32,
) -> Result<(u32, u32), String> {
    if w == 0 || h == 0 || max_w == 0 || max_h == 0 || dpi_x == 0 || dpi_y == 0 {
        return Err("Invalid print dimensions.".into());
    }
    let ratio = (w as f64 * dpi_x as f64) / (h as f64 * dpi_y as f64);
    let width = (max_w as f64).min(max_h as f64 * ratio);
    Ok((
        width.floor().max(1.0) as u32,
        (width / ratio).floor().max(1.0) as u32,
    ))
}

fn composite_white(frame: &mut Frame) {
    for pixel in frame.pixels.chunks_exact_mut(4) {
        let white = 255 - pixel[3];
        for channel in &mut pixel[..3] {
            *channel = channel.saturating_add(white);
        }
        pixel[3] = 255;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn print_geometry_and_ranges_do_not_need_a_printer() {
        assert_eq!(checked_range(2, 5, 9).unwrap(), (1, 4));
        for range in [(0, 1, 1), (3, 2, 4), (1, 6, 5)] {
            assert!(checked_range(range.0, range.1, range.2).is_err());
        }
        assert_eq!(render_bounds(4800, 6600, 600, 600).unwrap(), (2400, 3300));
        let bounds = render_bounds(100_000, 100_000, 300, 300).unwrap();
        assert!(frame_bytes(bounds.0, bounds.1).unwrap() <= MAX_FRAME_BYTES);
        assert_eq!(
            print_size(100, 100, 600, 300, 600, 300).unwrap(),
            (600, 300)
        );
        assert!(print_size(0, 100, 600, 300, 600, 300).is_err());
        let mut frame = Frame {
            width: 1,
            height: 1,
            pixels: vec![0, 0, 64, 128],
            page_count: 1,
            source_width: 1,
            source_height: 1,
        };
        composite_white(&mut frame);
        assert_eq!(frame.pixels, [127, 127, 191, 255]);
    }
}
