# Benchmarks

Status: the harness, suite, and gate run on Windows PowerShell 5.1. No baseline is recorded yet. Results below say "pending baseline".

## How to run

Build first: `cargo build --release`. Each run opens the app window for a moment. Leave the PC alone while a suite runs, and do not run it while code compiles.

| Task | Command | Time (estimate) |
| --- | --- | --- |
| Self-test with fake apps | `powershell -NoProfile -File tools/test-benchmark.ps1` | 15 s |
| One file | `powershell -NoProfile -File tools/benchmark.ps1 -Executable target/release/preview-for-windows.exe -InputFile fixtures/20-pages.pdf -Runs 30` | 20 s |
| Full suite | `powershell -NoProfile -File tools/perf/run-suite.ps1 -Runs 30` | 3 min |
| Gate (runs the suite) | `powershell -NoProfile -File tools/perf-gate.ps1 -Runs 30` | 3 min |
| Release gate | `powershell -NoProfile -File tools/perf-gate.ps1 -Runs 59` | 5 min |
| Compare an existing result | `powershell -NoProfile -File tools/perf-gate.ps1 -ResultsFile artifacts/perf/<time>/results.json` | 1 s |

To record a baseline:

1. Use a quiet PC: AC power, no builds or other heavy apps, the usual power plan.
2. Run `powershell -NoProfile -File tools/perf/run-suite.ps1 -Runs 59`.
3. Copy the `results.json` it prints to `benchmarks/baseline.json` and commit it with the build's commit ID in the message.

Times are estimates from a 5-run suite (22 s) and the self-test (12 s) on this PC. The suite reads `fixtures/`; pass `-FixturesDirectory` to use another folder with the same file names.

Output goes under `artifacts/` (not in git). Each `benchmark.ps1` call writes `samples.json`, `samples.csv`, `summary.json`, and every app marker file to a new folder. The suite writes `results.json`, which has the same shape as `benchmarks/baseline.json`.

## Method

**Launch to first content.** The harness reads QPC (`Stopwatch.GetTimestamp`) just before `Process.Start` (CreateProcess, no shell). The app gets `PFW_BENCH_OUT=<marker path>` and `PFW_BENCH_AUTOCLOSE=1`. After the first successful content draw and a successful `DwmFlush`, the app writes `first_content_qpc`, `qpc_frequency`, `width`, `height`, and `page_count`, then closes (`src/shell.rs`, contract in `docs/contracts.md` "Measurement"). A run passes only when all of these are true:

- The app exits with code 0 within the timeout (15 s). On timeout, the harness ends the process.
- The marker exists and has numeric fields.
- `qpc_frequency` equals the harness frequency, and `first_content_qpc` is between the launch QPC and the time of the check.
- `width`, `height`, and `page_count` are greater than 0.

**Statistics.** The harness keeps every raw sample. It uses nearest rank: the value at position ceil(p/100 × n) of the sorted samples. It reports p95, median (nearest-rank p50), min, and max.

**Run count.** The default is 30. Release gates use 59. Below 59 runs, no distribution-free 95% upper confidence bound on p95 exists. With 59 runs, that bound is the largest sample (`docs/research/winui-performance.md`, section 7, computed with binomial order statistics). The gate compares nearest-rank p95; `summary.json` also keeps the max. The harness does not drop a warm-up run. Run 1 can include file-cache effects; with 30 runs, p95 is the 29th sample, so one slow run does not move it.

**Memory.** The harness calls `GetProcessMemoryInfo` with `PROCESS_MEMORY_COUNTERS_EX`. Private bytes is `PrivateUsage` (private commit). Working set is `WorkingSetSize`. The two are always reported separately.

- Peak values (`PeakWorkingSetSize`, `PeakPagefileUsage`) are read after the app exits, while the harness still holds the process handle.
- Memory after idle (`-IdleSeconds 2`): the harness starts the app without `PFW_BENCH_AUTOCLOSE`, waits for the marker file, waits 2 s more, reads `WorkingSetSize` and `PrivateUsage`, then sends `WM_CLOSE` with `Process.CloseMainWindow` (a window message, not synthetic input). The app must exit with code 0.

**Sizes.** The executable size is the byte length of `target/release/preview-for-windows.exe`. The package size is `zip_bytes` from the JSON that `tools/package.ps1` prints.

**Gate.** `tools/perf-gate.ps1` runs the suite (or reads `-ResultsFile`), compares each metric with `benchmarks/baseline.json`, and prints one table: metric, baseline, current, change, PRD target, result. It exits 1 if a metric that has a baseline value:

- is more than 10% higher than the baseline (exactly 10% passes),
- has no current value (a failed or skipped measurement), or
- is missing from the results.

A metric with a null baseline shows "no baseline" and does not fail. The PRD target column is for reference only, because this PC is not the reference laptop. Sizes in the table use 1 MB = 1,000,000 bytes.

## Environment

| Item | This PC | PRD reference laptop |
| --- | --- | --- |
| OS | Windows 10 Home 22H2, build 19045 | Windows 11 |
| CPU | Intel i7-11800H (8 cores, 16 threads) | Intel Core i5 12th gen |
| RAM | 32 GB | 8 GB (low-end check: 4 GB, eMMC) |
| GPU | NVIDIA RTX 3050 Ti Laptop + Intel UHD | not stated |
| Shell | Windows PowerShell 5.1 (.NET Framework 4.8) | not applicable |

Numbers from this PC do not pass or fail the PRD gates. `results.json` records the OS, CPU, RAM, commit, and executable SHA-256 of each run.

## Metrics

| Metric | Definition | Gate statistic | PRD target |
| --- | --- | --- | --- |
| `launch_20_pages_pdf_p95` | Launch to first content, `fixtures/20-pages.pdf` | p95, ms | Cold launch to first page: p95 under 400 ms |
| `launch_500_pages_pdf_p95` | Launch to first content, `fixtures/500-pages-50mb.pdf` | p95, ms | Open 500-page, 50 MB PDF: first page under 300 ms |
| `launch_image_24mp_p95` | Launch to first content, `fixtures/image-24mp.jpg` | p95, ms | Cold launch to first image: p95 under 400 ms |
| `launch_image_small_p95` | Launch to first content, `fixtures/image-small.png` | p95, ms | Cold launch to first image: p95 under 400 ms |
| `memory_20_pages_pdf_private_bytes_max` | Private bytes 2 s after first content, `fixtures/20-pages.pdf` | max over runs, bytes | One 20-page PDF open: under 120 MB |
| `memory_20_pages_pdf_working_set_max` | Working set 2 s after first content, same runs | max over runs, bytes | One 20-page PDF open: under 120 MB |
| `exe_bytes` | Size of the release executable | bytes | None; part of the 30 MB download |
| `package_zip_bytes` | ZIP from `tools/package.ps1`, without the AI pack | bytes | Installer download: under 30 MB |

Notes on the mapping:

- Launch metrics include process creation, so the 500-page row is stricter than "open" in the PRD.
- The PRD does not define when memory is read. This suite reads it 2 s after first content, with no user input.
- Planned metrics need new app markers (see "Marker contracts for wave 2"): next image (PRD: under 50 ms), scrolling (PRD: 60 fps, no blank tiles after 100 ms), and resident handoff (PRD warm launch: p95 under 150 ms).

## Limitations

- **No cold launch.** A true cold launch needs a reboot or a standby-list flush, and a flush needs admin rights. The harness does neither. Every launch number is a process relaunch with the OS file cache warm, so it is a lower bound on cold launch.
- **First content is a proxy.** The marker is written after `DwmFlush` returns. That shows DWM composed the frame, not that the display scanned it out. Add up to one vsync (research estimate).
- **Not the reference hardware.** See "Environment".
- **Synthetic fixtures.** The 500-page PDF reaches 50 MB with whitespace padding. It is not complex production content.
- **Package size depends on `tools/package.ps1`.** That script currently starts with `#requires -Version 7.0`, and PowerShell 7 is not installed. Until W1-D ports it to 5.1, the suite records `package_zip_bytes` as not measured, with the reason. The package is built from `target/release`, not from `-Executable`.
- **Harness load.** The harness waits on the process handle in 10 ms steps (a kernel wait, not a busy loop). In idle mode, it checks for the marker file every 10 ms.
- **Contended runs are not baselines.** Measurements taken while other builds run are only harness checks.

## Results

Pending baseline. The orchestrator records the first baseline on a quiet PC with `-Runs 59`.

| Metric | Baseline | PRD target |
| --- | --- | --- |
| `launch_20_pages_pdf_p95` | pending baseline | under 400 ms |
| `launch_500_pages_pdf_p95` | pending baseline | under 300 ms |
| `launch_image_24mp_p95` | pending baseline | under 400 ms |
| `launch_image_small_p95` | pending baseline | under 400 ms |
| `memory_20_pages_pdf_private_bytes_max` | pending baseline | under 120 MB |
| `memory_20_pages_pdf_working_set_max` | pending baseline | under 120 MB |
| `exe_bytes` | pending baseline | none |
| `package_zip_bytes` | pending baseline | under 30 MB |

## Marker contracts for wave 2

These contracts let the harness measure next-image latency, scrolling, and resident handoff. Wave-2 slices add them to the app (W2-1 for next image and scrolling, W2-7 for handoff). Then add a suite step and a baseline metric for each.

### Rules for every marker

- The app reads the variables once at startup. When they are not set, the app does no benchmark work and writes no files.
- QPC values come from `QueryPerformanceCounter` as i64. `qpc_frequency` comes from `QueryPerformanceFrequency`.
- "Shown" means the same as for first content: the frame with the new content drew without error, then `DwmFlush` returned without error, then the app read QPC.
- Write each file atomically: write `<path>.tmp`, then rename it over `<path>` with `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`. The harness must never read a partial file.
- If a scenario fails, write `"status": "error"` with a plain-English `error`. Never write `"status": "ok"` for a blank or failed draw.
- Drive the app through the same command functions that keys and the mouse call. Do not post synthetic input (`SendInput`, `WM_KEYDOWN`, `WM_MOUSEWHEEL`).
- The first-content marker in `PFW_BENCH_OUT` does not change. A scenario starts after that marker is written.
- After the scenario file is written, close the window if `PFW_BENCH_AUTOCLOSE=1`.

### Scenario variables

| Variable | Value |
| --- | --- |
| `PFW_BENCH_SCENARIO` | `next-image` or `scroll` |
| `PFW_BENCH_SCENARIO_OUT` | Path of the scenario result file |
| `PFW_BENCH_STEPS` | `next-image` only: number of steps, an integer from 1 to 1000 |

### Next image (`PFW_BENCH_SCENARIO=next-image`)

Harness: it copies `fixtures/image-24mp.jpg` into a new folder as `001.jpg` to `<N+1>.jpg`, so the steps never reach the end of the folder. It starts the app on `001.jpg` with `PFW_BENCH_OUT`, `PFW_BENCH_SCENARIO=next-image`, `PFW_BENCH_STEPS=N`, `PFW_BENCH_SCENARIO_OUT`, and `PFW_BENCH_AUTOCLOSE=1`.

App, for each step from 1 to N:

1. Wait 500 ms with no work queued by the benchmark, so neighbor pre-decode can run, as it would while a person looks at a photo.
2. Read `start_qpc`, then call the same "next image" command as the Right arrow key.
3. When the next image is shown at its normal fit-to-window view, read `shown_qpc`.

```json
{
  "scenario": "next-image",
  "status": "ok",
  "error": null,
  "qpc_frequency": 10000000,
  "idle_before_step_ms": 500,
  "steps": [
    { "index": 1, "file": "C:\\bench\\002.jpg", "start_qpc": 81234567890, "shown_qpc": 81234789012, "source_width": 6000, "source_height": 4000, "from_predecode": true }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `steps[].file` | Full path of the image shown by this step |
| `steps[].source_width`, `source_height` | Pixel size of the source image, from the decoder |
| `steps[].from_predecode` | True if the shown bitmap came from the pre-decode cache |

Harness metric: `next_image_24mp_p95` is the nearest-rank p95 of `1000 × (shown_qpc − start_qpc) / qpc_frequency` over all steps of all runs. PRD target: under 50 ms.

### Scrolling (`PFW_BENCH_SCENARIO=scroll`)

Harness: it starts the app on `fixtures/500-pages-50mb.pdf` with `PFW_BENCH_OUT`, `PFW_BENCH_SCENARIO=scroll`, `PFW_BENCH_SCENARIO_OUT`, and `PFW_BENCH_AUTOCLOSE=1`. It does not resize the window.

App:

1. After first content, set zoom to fit width and scroll to the top.
2. For 5000 ms, scroll down at 2 viewport heights per second through the same function that the mouse wheel and scroll bar use. Move by elapsed time × speed on each frame, so a slow frame jumps further, as real scrolling does.
3. Present frames as in normal use (no vsync change). For each frame, record one entry.

```json
{
  "scenario": "scroll",
  "status": "ok",
  "error": null,
  "qpc_frequency": 10000000,
  "duration_ms": 5000,
  "speed_viewports_per_second": 2,
  "viewport_width": 1280,
  "viewport_height": 720,
  "dpi": 96,
  "refresh_hz": 60,
  "frames": [
    { "present_qpc": 81234567890, "scroll_y": 1440.0, "visible_tiles": 12, "blank_tiles": 0, "oldest_blank_ms": 0.0 }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `viewport_width`, `viewport_height` | Page view size in device pixels |
| `refresh_hz` | Display refresh rate from `DwmGetCompositionTimingInfo` (`rateRefresh`) |
| `frames[].present_qpc` | QPC right after the frame's present call returns. No `DwmFlush` per frame. |
| `frames[].scroll_y` | Distance from the top of the document to the top of the view, in DIPs |
| `frames[].visible_tiles` | Tiles that overlap the view in this frame |
| `frames[].blank_tiles` | Visible tiles with no rendered pixels at any scale. A lower-resolution placeholder is not blank. |
| `frames[].oldest_blank_ms` | Age of the oldest blank visible tile, from the first frame in which it was visible; 0 if none |

Harness metrics: `scroll_frame_p95` is the nearest-rank p95 of the time between consecutive `present_qpc` values (PRD 60 fps: 16.7 ms or less). `scroll_blank_max` is the largest `oldest_blank_ms` (PRD: 100 ms or less).

### Resident handoff (warm open through a running instance)

The PRD calls this "warm launch". The harness names it `resident-handoff`, because the file opens in a process that already runs. `process-relaunch` stays the name for a new process that opens its own window.

| Variable | Set on | Value |
| --- | --- | --- |
| `PFW_BENCH_HANDOFF_OUT` | The first instance only | Path of the handoff result file |

Harness:

1. Start instance A on `fixtures/image-24mp.jpg` with `PFW_BENCH_OUT` and `PFW_BENCH_HANDOFF_OUT`, without `PFW_BENCH_AUTOCLOSE`. Wait for the first-content marker.
2. For each run: delete the handoff file, read T0 from QPC, then start instance B with one file. Alternate between `fixtures/image-small.png` and `fixtures/20-pages.pdf`, so each handoff changes the content. B gets no benchmark variables.
3. Wait for the handoff file (timeout 15 s). Check that B exited with code 0, that `file` matches B's file, and that `handoff_index` equals the run number.
4. After the last run, send `WM_CLOSE` to A and check that A exits with code 0.

App A: each time a file from another instance is shown in A's window (a new tab, or a switch to a tab that already has the file), A writes:

```json
{ "shown_qpc": 81234567890, "qpc_frequency": 10000000, "file": "C:\\...\\image-small.png", "width": 64, "height": 64, "page_count": 1, "handoff_index": 3 }
```

`handoff_index` counts the handoffs A received since it started, from 1. `width`, `height`, and `page_count` follow the first-content marker. If B finds no running instance, it opens its own window, writes nothing, and the harness run times out, so the run fails.

Harness metric: `resident_handoff_p95` is the nearest-rank p95 of `1000 × (shown_qpc − T0) / qpc_frequency`. It includes B's process start, the single-instance check, the handoff message, and A's load and draw. PRD target (warm launch): p95 under 150 ms.
