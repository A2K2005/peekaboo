#![allow(dead_code)]
#[path = "../src/integration.rs"]
mod integration;

use integration::{Action, Command};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

static RECEIVED: Mutex<Vec<(Instant, Command)>> = Mutex::new(Vec::new());

extern "system" fn receiver(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match message {
            WM_COPYDATA => match integration::decode_copydata(lparam) {
                Some(command) => {
                    RECEIVED.lock().unwrap().push((Instant::now(), command));
                    LRESULT(1)
                }
                None => LRESULT(0),
            },
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

/// A receiver window with its own message loop, like a running Peekaboo.
struct Receiver {
    hwnd: isize,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Receiver {
    fn start(class: &str) -> Receiver {
        let class = class.to_owned();
        let (send, receive) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || unsafe {
            let name = HSTRING::from(class);
            let instance = GetModuleHandleW(None).unwrap();
            let wc = WNDCLASSW {
                hInstance: instance.into(),
                lpszClassName: PCWSTR(name.as_ptr()),
                lpfnWndProc: Some(receiver),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&wc), 0);
            // Message-only: it can never be shown, so the test stays headless.
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                &name,
                &name,
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance.into()),
                None,
            )
            .unwrap();
            send.send(hwnd.0 as isize).unwrap();
            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                DispatchMessageW(&message);
            }
        });
        Receiver {
            hwnd: receive.recv().unwrap(),
            thread: Some(thread),
        }
    }

    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut _)
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        self.thread.take().unwrap().join().unwrap();
    }
}

fn unique_class(name: &str) -> String {
    format!("PeekabooTest.{name}.{}", std::process::id())
}

fn percentile(samples: &mut [f64], p: f64) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let rank = ((p / 100.0) * samples.len() as f64).ceil().max(1.0) as usize;
    samples[rank - 1]
}

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

#[test]
fn parse_args_reads_verb_and_absolute_paths() {
    let here = std::env::current_dir().unwrap();
    let command = integration::parse_args(os(&[
        "--convert",
        "a.jpg",
        r"C:\x\b.png",
        "--quiet",
        "a.jpg",
    ]));
    assert_eq!(command.action, Action::Convert);
    assert_eq!(
        command.paths,
        vec![here.join("a.jpg"), PathBuf::from(r"C:\x\b.png")]
    );

    let command = integration::parse_args(os(&[r"C:\x\b.png", "--resize"]));
    assert_eq!(
        command,
        Command {
            action: Action::Open,
            paths: vec![PathBuf::from(r"C:\x\b.png")]
        }
    );
    assert_eq!(
        integration::parse_args(Vec::new()),
        Command {
            action: Action::Open,
            paths: Vec::new()
        }
    );
    assert_eq!(
        integration::parse_args(os(&["--combine"])).action,
        Action::Combine
    );
}

#[test]
fn merge_collects_one_action_without_duplicates() {
    let a = PathBuf::from(r"C:\a.jpg");
    let b = PathBuf::from(r"C:\b.jpg");
    let mut pending = Command {
        action: Action::Convert,
        paths: vec![a.clone()],
    };
    assert!(integration::merge(
        &mut pending,
        Command {
            action: Action::Convert,
            paths: vec![b.clone(), a.clone()]
        }
    ));
    assert_eq!(pending.paths, vec![a.clone(), b.clone()]);
    assert!(!integration::merge(
        &mut pending,
        Command {
            action: Action::Resize,
            paths: vec![PathBuf::from(r"C:\c.jpg")]
        }
    ));
    assert_eq!(pending.paths, vec![a, b]);
}

#[test]
fn decode_accepts_only_tagged_absolute_paths() {
    let command = Command {
        action: Action::Resize,
        paths: vec![
            PathBuf::from(r"C:\Photos\a b.jpg"),
            PathBuf::from(r"\\server\share\c.png"),
        ],
    };
    let data = integration::encode(&command);
    assert_eq!(integration::decode(&data), Some(command.clone()));

    // Relative and device paths are dropped; verbatim drive paths stay.
    let mut mixed = integration::encode(&Command {
        action: Action::Open,
        paths: Vec::new(),
    });
    for path in [
        r"relative.jpg",
        r"\\.\PhysicalDrive0",
        r"\\?\C:\long\d.pdf",
        r"C:no-root.pdf",
        r"\rooted.pdf",
    ] {
        mixed.extend(path.encode_utf16().chain([0]));
    }
    assert_eq!(
        integration::decode(&mixed),
        Some(Command {
            action: Action::Open,
            paths: vec![PathBuf::from(r"\\?\C:\long\d.pdf")]
        })
    );

    assert_eq!(integration::decode(&[]), None);
    assert_eq!(
        integration::decode(&data[..data.len() - 1]),
        None,
        "missing final NUL"
    );
    assert_eq!(
        integration::decode(&"--delete\0".encode_utf16().collect::<Vec<_>>()),
        None
    );

    let decode_struct = |data: &COPYDATASTRUCT| unsafe {
        integration::decode_copydata(LPARAM(data as *const COPYDATASTRUCT as isize))
    };
    let good = COPYDATASTRUCT {
        dwData: integration::COPYDATA_TAG,
        cbData: (data.len() * 2) as u32,
        lpData: data.as_ptr() as *mut _,
    };
    assert_eq!(decode_struct(&good), Some(command));
    assert_eq!(
        decode_struct(&COPYDATASTRUCT { dwData: 7, ..good }),
        None,
        "wrong tag"
    );
    assert_eq!(
        decode_struct(&COPYDATASTRUCT {
            cbData: good.cbData - 1,
            ..good
        }),
        None,
        "odd size"
    );
    assert_eq!(
        decode_struct(&COPYDATASTRUCT { cbData: 0, ..good }),
        None,
        "empty"
    );
    assert_eq!(
        decode_struct(&COPYDATASTRUCT {
            cbData: 2 << 20,
            ..good
        }),
        None,
        "too large"
    );
    assert_eq!(
        decode_struct(&COPYDATASTRUCT {
            lpData: std::ptr::null_mut(),
            ..good
        }),
        None,
        "null data"
    );
    assert_eq!(
        unsafe { integration::decode_copydata(LPARAM(0)) },
        None,
        "null struct"
    );
}

#[test]
fn forward_reaches_running_window() {
    let class = unique_class("forward");
    let window = Receiver::start(&class);
    assert_eq!(integration::find_window(&class), Some(window.hwnd()));
    let command = Command {
        action: Action::Combine,
        paths: vec![PathBuf::from(r"C:\a.pdf"), PathBuf::from(r"C:\b.jpg")],
    };
    RECEIVED.lock().unwrap().clear();
    let mut samples = Vec::new();
    for _ in 0..200 {
        let start = Instant::now();
        integration::forward(window.hwnd(), &command).unwrap();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let received = RECEIVED
        .lock()
        .unwrap()
        .drain(..)
        .map(|(_, c)| c)
        .collect::<Vec<_>>();
    assert_eq!(received, vec![command; 200]);
    let (p50, p95) = (
        percentile(&mut samples, 50.0),
        percentile(&mut samples, 95.0),
    );
    println!("WM_COPYDATA send and decode, 200 runs: p50 {p50:.3} ms, p95 {p95:.3} ms");
    assert!(p95 < 150.0);
}

#[test]
fn hand_off_from_a_new_process_meets_warm_open_target() {
    let class = unique_class("process");
    let primary = integration::hand_off(
        &class,
        &Command {
            action: Action::Open,
            paths: Vec::new(),
        },
    );
    assert!(
        primary.is_some(),
        "the first process must become the primary"
    );
    let window = Receiver::start(&class);
    let file = std::env::current_exe().unwrap();
    RECEIVED.lock().unwrap().clear();
    let mut samples = Vec::new();
    for _ in 0..30 {
        let start = Instant::now();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "hand_off_child",
                "--ignored",
                "--test-threads=1",
                "--quiet",
            ])
            .env("PFW_HANDOFF_CLASS", &class)
            .env("PFW_HANDOFF_FILE", &file)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "the child process must forward and exit");
        let (at, command) = RECEIVED
            .lock()
            .unwrap()
            .pop()
            .expect("the window received nothing");
        assert_eq!(
            command,
            Command {
                action: Action::Convert,
                paths: vec![file.clone()]
            }
        );
        samples.push((at - start).as_secs_f64() * 1000.0);
    }
    let (p50, p95) = (
        percentile(&mut samples, 50.0),
        percentile(&mut samples, 95.0),
    );
    println!("Process start to WM_COPYDATA received, 30 runs: p50 {p50:.1} ms, p95 {p95:.1} ms");
    assert!(p95 < 150.0, "warm open target is p95 under 150 ms");
    drop(window);
    drop(primary);
}

/// Child process for the test above. It does nothing when run directly.
#[test]
#[ignore = "child process of hand_off_from_a_new_process_meets_warm_open_target"]
fn hand_off_child() {
    let (Some(class), Some(file)) = (
        std::env::var_os("PFW_HANDOFF_CLASS"),
        std::env::var_os("PFW_HANDOFF_FILE"),
    ) else {
        return;
    };
    let command = integration::parse_args([OsString::from("--convert"), file]);
    let guard = integration::hand_off(class.to_str().unwrap(), &command);
    assert!(
        guard.is_none(),
        "the running window should have taken the files"
    );
}

#[test]
fn hand_off_opens_its_own_window_when_the_primary_never_answers() {
    let class = unique_class("silent");
    let command = Command {
        action: Action::Open,
        paths: Vec::new(),
    };
    let primary = integration::hand_off(&class, &command);
    assert!(primary.is_some());
    // The primary holds the name but never creates a window.
    let start = Instant::now();
    let second = integration::hand_off(&class, &command);
    assert!(second.is_some(), "the request must not be lost");
    assert!(start.elapsed() >= Duration::from_secs(5));
    drop(second);
    drop(primary);
    assert!(integration::hand_off(&class, &command).is_some());
}
