#!/usr/bin/env python3
"""KntonOS Headless Boot Test Runner.

Executes QEMU with OVMF firmware, boots kboot.efi, monitors serial output for
sentinel tokens, verifies success criteria, and exits with 0 (pass) or 1 (fail).
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import time
import threading
import queue

SENTINEL_SUCCESS = re.compile(r"\[KNT-BOOT-SUCCESS\]")
SENTINEL_PANIC = re.compile(r"\[KNT-PANIC\]")
SENTINEL_TEST_FAILURE = re.compile(r"\[KNT-TEST-FAIL\]")
SENTINEL_UEFI = re.compile(r"\[KNT-UEFI-START\]")
SENTINEL_KERNEL = re.compile(r"\[KNT-KERNEL-INIT\]")

DEFAULT_SEARCH_PATHS = [
    r"C:\Users\Kingo\qemu",
    r"C:\Program Files\qemu",
    r"C:\qemu",
]

def find_qemu_executable() -> str:
    which_path = shutil.which("qemu-system-x86_64")
    if which_path:
        return which_path
    for p in DEFAULT_SEARCH_PATHS:
        exe = os.path.join(p, "qemu-system-x86_64.exe")
        if os.path.isfile(exe):
            return exe
    raise FileNotFoundError("Could not find qemu-system-x86_64 in PATH or standard locations.")

def find_ovmf_firmware(qemu_exe: str) -> str:
    qemu_dir = os.path.dirname(qemu_exe)
    candidates = [
        os.path.join(qemu_dir, "share", "edk2-x86_64-code.fd"),
        os.path.join(qemu_dir, "edk2-x86_64-code.fd"),
        os.path.join(qemu_dir, "share", "qemu", "edk2-x86_64-code.fd"),
        r"C:\Users\Kingo\qemu\share\edk2-x86_64-code.fd",
    ]
    for c in candidates:
        if os.path.isfile(c):
            return c
    # Fallback to search recursively in qemu_dir
    for root, _, files in os.walk(qemu_dir):
        for f in files:
            if "edk2-x86_64-code.fd" in f or "OVMF_CODE" in f:
                return os.path.join(root, f)
    raise FileNotFoundError(f"Could not locate OVMF firmware in {qemu_dir}")

def run_boot_test(efi_path: str, timeout_sec: int = 30) -> int:
    if not os.path.isfile(efi_path):
        print(f"[HARNESS-ERROR] EFI executable not found: {efi_path}", file=sys.stderr)
        return 1

    try:
        qemu_exe = find_qemu_executable()
        ovmf_fw = find_ovmf_firmware(qemu_exe)
    except FileNotFoundError as err:
        print(f"[HARNESS-ERROR] {err}", file=sys.stderr)
        return 1

    print(f"[HARNESS] Using QEMU: {qemu_exe}")
    print(f"[HARNESS] Using OVMF: {ovmf_fw}")

    # Create standard EFI System Partition structure: EFI/BOOT/BOOTX64.EFI
    workspace_root = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
    esp_dir = os.path.join(workspace_root, "target", "esp")
    boot_dir = os.path.join(esp_dir, "EFI", "BOOT")
    os.makedirs(boot_dir, exist_ok=True)

    dest_efi = os.path.join(boot_dir, "BOOTX64.EFI")
    shutil.copy2(efi_path, dest_efi)
    print(f"[HARNESS] Staged {dest_efi} ({os.path.getsize(dest_efi)} bytes)")

    cmd = [
        qemu_exe,
        "-drive", f"if=pflash,format=raw,readonly=on,file={ovmf_fw}",
        "-drive", f"file=fat:rw:{esp_dir},format=raw,media=disk",
        "-machine", "q35,accel=tcg",
        "-cpu", "max",
        "-m", "512M",
        "-display", "none",
        "-serial", "stdio",
        "-no-reboot",
    ]

    print(f"[HARNESS] Executing QEMU with timeout of {timeout_sec}s...")
    start_time = time.time()
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )

    uefi_started = False
    kernel_initialized = False
    boot_success = False
    panic_detected = False
    test_failed = False

    q = queue.Queue()
    def enqueue_output(out, queue_obj):
        try:
            for l in iter(out.readline, ''):
                queue_obj.put(l)
        except Exception:
            pass
        finally:
            out.close()

    t = threading.Thread(target=enqueue_output, args=(proc.stdout, q), daemon=True)
    t.start()

    try:
        while True:
            elapsed = time.time() - start_time
            if elapsed > timeout_sec:
                print(f"\n[HARNESS] Timeout exceeded ({timeout_sec}s). Stopping QEMU.", file=sys.stderr)
                break

            try:
                line = q.get(timeout=0.1)
            except queue.Empty:
                if proc.poll() is not None:
                    break
                continue

            if line:
                sys.stdout.write(f"[SERIAL] {line}")
                sys.stdout.flush()

                if SENTINEL_UEFI.search(line):
                    uefi_started = True
                if SENTINEL_KERNEL.search(line):
                    kernel_initialized = True
                if SENTINEL_PANIC.search(line):
                    panic_detected = True
                if SENTINEL_TEST_FAILURE.search(line):
                    test_failed = True
                if SENTINEL_SUCCESS.search(line):
                    boot_success = True
                    # Let it run briefly to ensure clean halt
                    break
    finally:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()

    print("\n--- Boot Test Execution Summary ---")
    print(f"Elapsed Time: {time.time() - start_time:.2f}s")
    print(f"UEFI Initialized:      {'YES' if uefi_started else 'NO'}")
    print(f"Kernel Bootstrap:      {'YES' if kernel_initialized else 'NO'}")
    print(f"Boot Success Sentinel: {'PASS' if boot_success else 'FAIL'}")

    if panic_detected:
        print("[HARNESS] RESULT: FAILED (Kernel panic detected)", file=sys.stderr)
        return 1
    if test_failed:
        print("[HARNESS] RESULT: FAILED (Kernel self-test failure detected)", file=sys.stderr)
        return 1
    if not uefi_started or not kernel_initialized:
        print("[HARNESS] RESULT: FAILED (Missing UEFI or kernel initialization sentinel)", file=sys.stderr)
        return 1
    if not boot_success:
        print("[HARNESS] RESULT: FAILED (Did not reach [KNT-BOOT-SUCCESS])", file=sys.stderr)
        return 1

    print("[HARNESS] RESULT: PASSED (Native UEFI-to-kernel boot verified)")
    return 0

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="KntonOS Automated Boot Runner")
    parser.add_argument(
        "--efi",
        default=os.path.join(
            os.path.dirname(__file__),
            "..",
            "target",
            "x86_64-unknown-uefi",
            "debug",
            "kboot.efi",
        ),
        help="Path to kboot.efi executable",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=30,
        help="Timeout in seconds (default: 30)",
    )
    args = parser.parse_args()
    sys.exit(run_boot_test(os.path.abspath(args.efi), args.timeout))
