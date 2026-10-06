"""Shared launch benchmark for the ui-stack "hello window" proofs.

Contract with each benchmark app:
  The harness sets PFW_BENCH_OUT to a file path. When the app's first frame is
  on screen (first Present done, then DwmFlush() returned), the app writes the
  QueryPerformanceCounter value as decimal text to that file. The app keeps
  running; the harness kills it.

QPC values are comparable across processes on one PC (Microsoft Learn,
"Acquiring high-resolution time stamps"), so launch time is
(app QPC - harness QPC taken just before CreateProcess) / QPC frequency.

Usage:
  python measure.py --exe PATH [--label NAME] [--runs 20] [--idle-ms 2000]
                    [--cold] [--csv results.csv]
  python measure.py --selftest

--cold flushes the modified page list and purges the standby list before every
run, so the app's and the framework's DLLs come from disk again. It needs an
elevated prompt. A reboot before each run is the gold standard; --cold is the
repeatable proxy.
"""
import argparse
import csv
import ctypes
import math
import os
import statistics
import subprocess
import sys
import tempfile
import time
from ctypes import wintypes

k32 = ctypes.WinDLL("kernel32", use_last_error=True)
adv = ctypes.WinDLL("advapi32", use_last_error=True)
ntdll = ctypes.WinDLL("ntdll")


def qpc():
    v = ctypes.c_int64()
    k32.QueryPerformanceCounter(ctypes.byref(v))
    return v.value


def qpf():
    v = ctypes.c_int64()
    k32.QueryPerformanceFrequency(ctypes.byref(v))
    return v.value


class LUID(ctypes.Structure):
    _fields_ = [("LowPart", wintypes.DWORD), ("HighPart", wintypes.LONG)]


class TOKEN_PRIVILEGES(ctypes.Structure):
    _fields_ = [("PrivilegeCount", wintypes.DWORD), ("Luid", LUID), ("Attributes", wintypes.DWORD)]


class PROCESS_MEMORY_COUNTERS_EX(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [
        (n, ctypes.c_size_t) for n in (
            "PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage",
            "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage",
            "PagefileUsage", "PeakPagefileUsage", "PrivateUsage")]


def enable_privilege(name):
    token = wintypes.HANDLE()
    if not adv.OpenProcessToken(k32.GetCurrentProcess(), 0x0020 | 0x0008, ctypes.byref(token)):
        raise ctypes.WinError(ctypes.get_last_error())
    tp = TOKEN_PRIVILEGES(1, LUID(), 0x2)  # SE_PRIVILEGE_ENABLED
    if not adv.LookupPrivilegeValueW(None, name, ctypes.byref(tp.Luid)):
        raise ctypes.WinError(ctypes.get_last_error())
    adv.AdjustTokenPrivileges(token, False, ctypes.byref(tp), 0, None, None)
    if ctypes.get_last_error() != 0:  # ERROR_NOT_ALL_ASSIGNED: not elevated
        raise PermissionError(f"{name} not held; run from an elevated prompt")
    k32.CloseHandle(token)


def purge_standby():
    # Undocumented NtSetSystemInformation(SystemMemoryListInformation=80), the
    # same call RAMMap's "Empty Standby List" makes. 3 = flush modified list,
    # 4 = purge standby list.
    enable_privilege("SeProfileSingleProcessPrivilege")
    for command in (3, 4):
        c = ctypes.c_int(command)
        status = ntdll.NtSetSystemInformation(80, ctypes.byref(c), ctypes.sizeof(c))
        if status != 0:
            raise OSError(f"NtSetSystemInformation({command}) failed: 0x{status & 0xFFFFFFFF:08X}")


def memory_mb(handle):
    pmc = PROCESS_MEMORY_COUNTERS_EX()
    pmc.cb = ctypes.sizeof(pmc)
    if not k32.K32GetProcessMemoryInfo(wintypes.HANDLE(handle), ctypes.byref(pmc), pmc.cb):
        raise ctypes.WinError(ctypes.get_last_error())
    return pmc.WorkingSetSize / 2**20, pmc.PrivateUsage / 2**20


def one_run(cmd, freq, idle_ms, timeout_s=15.0):
    fd, out = tempfile.mkstemp(prefix="pfw-bench-")
    os.close(fd)
    os.remove(out)  # the app creates it
    env = dict(os.environ, PFW_BENCH_OUT=out)
    t0 = qpc()
    proc = subprocess.Popen(cmd, env=env)
    try:
        deadline = time.monotonic() + timeout_s
        t1 = None
        while t1 is None:
            if proc.poll() is not None:
                raise RuntimeError(f"app exited with code {proc.returncode} before first frame")
            if time.monotonic() > deadline:
                raise TimeoutError("no first-frame marker within timeout")
            try:
                with open(out) as f:
                    t1 = int(f.read().strip())
            except (OSError, ValueError):
                time.sleep(0.001)
        ws = priv = float("nan")
        if idle_ms:
            time.sleep(idle_ms / 1000)
            ws, priv = memory_mb(proc._handle)
        return (t1 - t0) * 1000 / freq, ws, priv
    finally:
        proc.kill()
        proc.wait()
        if os.path.exists(out):
            os.remove(out)


def p95(values):
    s = sorted(values)
    return s[max(0, math.ceil(0.95 * len(s)) - 1)]  # nearest-rank


def folder_mb(exe):
    root = os.path.dirname(os.path.abspath(exe))
    total = sum(os.path.getsize(os.path.join(d, f)) for d, _, fs in os.walk(root) for f in fs)
    return total / 2**20


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--exe")
    ap.add_argument("--label")
    ap.add_argument("--runs", type=int, default=20)
    ap.add_argument("--idle-ms", type=int, default=2000, help="wait after first frame, then read memory; 0 = skip")
    ap.add_argument("--cold", action="store_true", help="purge standby list before each run (admin)")
    ap.add_argument("--csv")
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()

    if a.selftest:
        # A child that writes its own QPC at once must report a small positive time.
        child = ("import ctypes,os;v=ctypes.c_int64();"
                 "ctypes.windll.kernel32.QueryPerformanceCounter(ctypes.byref(v));"
                 "open(os.environ['PFW_BENCH_OUT'],'w').write(str(v.value));"
                 "import time;time.sleep(5)")
        ms, ws, _ = one_run([sys.executable, "-c", child], qpf(), idle_ms=100)
        assert 0 < ms < 5000, ms
        assert ws > 1, ws
        print(f"selftest ok: python child first marker after {ms:.1f} ms, working set {ws:.1f} MB")
        return

    if not a.exe:
        ap.error("--exe is required")
    label = a.label or os.path.basename(a.exe)
    freq = qpf()
    if not a.cold:
        one_run([a.exe], freq, 0)  # discarded warm-up run fills the file cache
    rows = []
    for i in range(1, a.runs + 1):
        if a.cold:
            purge_standby()
            time.sleep(1.0)  # let the disk settle after the purge
        ms, ws, priv = one_run([a.exe], freq, a.idle_ms)
        rows.append((label, "cold" if a.cold else "warm", i, round(ms, 2), round(ws, 1), round(priv, 1)))
        print(f"{label} run {i:2d}: {ms:7.1f} ms  ws {ws:6.1f} MB  private {priv:6.1f} MB")
        time.sleep(0.5)

    times = [r[3] for r in rows]
    mems = [r[4] for r in rows if not math.isnan(r[4])]
    print(f"\n{label} ({rows[0][1]}, n={len(times)}): min {min(times):.1f}  median {statistics.median(times):.1f}"
          f"  p95 {p95(times):.1f}  max {max(times):.1f} ms")
    if mems:
        print(f"working set after {a.idle_ms} ms idle: median {statistics.median(mems):.1f} MB")
    print(f"app folder size: {folder_mb(a.exe):.1f} MB")
    if a.csv:
        new = not os.path.exists(a.csv)
        with open(a.csv, "a", newline="") as f:
            w = csv.writer(f)
            if new:
                w.writerow(["label", "mode", "run", "launch_ms", "working_set_mb", "private_mb"])
            w.writerows(rows)


if __name__ == "__main__":
    main()
