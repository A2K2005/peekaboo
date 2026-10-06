// Not yet compiled: written before the Rust toolchain was installed.
// API shapes follow the windows-rs 0.62 Direct2D sample (release tag 71).
#![windows_subsystem = "windows"]

use std::cell::{Cell, RefCell};
use windows::{
    core::*, Win32::Foundation::*, Win32::Graphics::Direct2D::Common::*,
    Win32::Graphics::Direct2D::*, Win32::Graphics::Direct3D::*, Win32::Graphics::Direct3D11::*,
    Win32::Graphics::DirectWrite::*, Win32::Graphics::Dwm::DwmFlush,
    Win32::Graphics::Dxgi::Common::*, Win32::Graphics::Dxgi::*, Win32::Graphics::Gdi::ValidateRect,
    Win32::System::LibraryLoader::GetModuleHandleW,
    Win32::System::Performance::QueryPerformanceCounter, Win32::UI::HiDpi::*,
    Win32::UI::WindowsAndMessaging::*,
};

struct Gfx {
    dc: ID2D1DeviceContext,
    swapchain: IDXGISwapChain1,
    brush: ID2D1SolidColorBrush,
    format: IDWriteTextFormat,
}

thread_local! {
    static GFX: RefCell<Option<Gfx>> = const { RefCell::new(None) };
    static MARKED: Cell<bool> = const { Cell::new(false) };
}

fn main() -> Result<()> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        let instance = GetModuleHandleW(None)?;
        let class = w!("PfwBench");
        let wc = WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: class,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        };
        RegisterClassW(&wc);
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!("PFW bench: Rust Win32 Direct2D"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1280, // same size in every proof
            800,
            None,
            None,
            None,
            None,
        )?;
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                if paint(hwnd).is_ok() && !MARKED.replace(true) {
                    mark_first_frame();
                }
                let _ = ValidateRect(Some(hwnd), None);
                LRESULT(0)
            }
            // ponytail: rebuilds all GPU objects on resize; resize swap chain buffers if it matters.
            WM_SIZE => {
                GFX.with(|g| {
                    if let Ok(mut g) = g.try_borrow_mut() {
                        *g = None; // try_: DXGI may send WM_SIZE while paint() holds the borrow
                    }
                });
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn paint(hwnd: HWND) -> Result<()> {
    GFX.with_borrow_mut(|slot| {
        if slot.is_none() {
            *slot = Some(Gfx::new(hwnd)?);
        }
        let g = slot.as_ref().unwrap();
        let text: Vec<u16> = "Hello".encode_utf16().collect();
        unsafe {
            g.dc.BeginDraw();
            g.dc.Clear(Some(&D2D1_COLOR_F { r: 0.953, g: 0.953, b: 0.953, a: 1.0 }));
            g.dc.DrawText(
                &text,
                &g.format,
                &D2D_RECT_F { left: 24.0, top: 24.0, right: 600.0, bottom: 100.0 },
                &g.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            g.dc.EndDraw(None, None)?;
            g.swapchain.Present(1, DXGI_PRESENT(0)).ok()
        }
    })
}

impl Gfx {
    fn new(hwnd: HWND) -> Result<Self> {
        unsafe {
            let mut device = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )?;
            let device: ID3D11Device = device.unwrap();
            let dxgi_device: IDXGIDevice = device.cast()?;

            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dc = factory.CreateDevice(&dxgi_device)?.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let dpi = GetDpiForWindow(hwnd) as f32;
            dc.SetDpi(dpi, dpi);

            let dxgi_factory: IDXGIFactory2 = dxgi_device.GetAdapter()?.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                ..Default::default()
            };
            let swapchain = dxgi_factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None)?;
            let surface: IDXGISurface = swapchain.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
                dpiX: dpi,
                dpiY: dpi,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..Default::default()
            };
            dc.SetTarget(&dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?);

            let brush = dc.CreateSolidColorBrush(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let format = dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                24.0,
                w!("en-us"),
            )?;
            Ok(Gfx { dc, swapchain, brush, format })
        }
    }
}

// Benchmark marker (see ../measure.py): wait for DWM to present the frame, then write the QPC value.
fn mark_first_frame() {
    let Ok(path) = std::env::var("PFW_BENCH_OUT") else { return };
    let mut ticks = 0i64;
    unsafe {
        let _ = DwmFlush();
        let _ = QueryPerformanceCounter(&mut ticks);
    }
    let _ = std::fs::write(path, ticks.to_string());
}
