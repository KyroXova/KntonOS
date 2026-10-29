# RESEARCH-001: x86-64 UEFI Toolchain & Emulation Validation Fixtures

- **Document ID**: RESEARCH-001
- **Related Tasks**: TASK-KNT-003 (Platform & Compatibility ADR), TASK-KNT-004 (Performance Plan)
- **Author**: @JulesMartin (Developer, Knton OS)
- **Target Workspace**: `workspace/jules-martin` (`E:\Git Lab's\kntonOS-jules-martin`)
- **Status**: Complete / Ready for Review

---

## 1. Executive Summary

KntonOS requires an independently developed, modern x86-64 UEFI operating system kernel with exceptional performance, low resource consumption, a Windows 11-like desktop interface, and support for Windows `.exe` binaries and games.

To establish early velocity while avoiding architectural dead-ends, this research artifact evaluates:
1. **Toolchain options** for the bare-metal kernel, early bootloader, and early user-space (Rust vs. modern C/C++ freestanding).
2. **Standardized QEMU + OVMF emulation fixtures** ensuring identical execution characteristics across host environments (Windows WHPX, Linux KVM, and CI headless runners).
3. **Automated headless CI test harnesses** with serial-port sentinel validation and deterministic timeout enforcement.
4. **Low-level debugging infrastructure**, including GDB/LLDB remote target attachment, IDT exception handling with dedicated IST stacks, and early-stage panic diagnostic formatting.

---

## 2. Toolchain Comparison: Bare-Metal Kernel & Early User-Space

Developing an independent operating system kernel from scratch requires choosing a language and toolchain that balance low-level hardware control, spatial/temporal memory safety, concurrency guarantees, and foreign function interface (FFI) interoperability with Windows PE/COFF structures and C-style calling conventions.

### 2.1 Candidate Evaluation

| Evaluation Criteria | Rust (`x86_64-unknown-none` / `uefi` / `bootloader`) | Freestanding C/C++ (`clang`/`gcc` -ffreestanding) |
| :--- | :--- | :--- |
| **Spatial & Temporal Safety** | **High**: Ownership and borrow checker eliminate use-after-free, double-free, and data races at compile time. `unsafe` is strictly scoped and auditable. | **Low**: Manual pointer management; vulnerability to buffer overruns, uninitialized reads, and temporal memory corruption unless mitigated by static analysis / sanitizers. |
| **Compile-Time Guarantees** | **Superior**: Algebraic data types, exhaustive pattern matching, strict null safety (`Option<T>`), RAII memory and lock cleanup. | **Moderate**: Type system lacks built-in borrow tracking; constexpr/templates offer metaprogramming but diagnostics can be fragile in freestanding mode. |
| **ABI Stability & Interop** | **Mixed / High with `extern "C"` / `extern "win64"`**: Internal Rust ABI is unstable, but native `extern "win64"` and `extern "sysv64"` calling conventions are supported. `#[repr(C, packed)]` maps directly to PE/COFF and UEFI structures. | **Native**: Direct, native C ABI compatibility. Seamless alignment with hardware calling conventions, MSVC x64 ABI (`__fastcall`), and PE32+ structures. |
| **UEFI Boot Ecosystem** | **Mature**: `uefi-rs` provides idiomatic, safe bindings to UEFI Boot Services, Runtime Services, and GOP protocols. `bootloader` crate offers automated stage-2 loading. | **Established**: GNU-EFI and EDK2 MdePkg are battle-tested, but introduce heavy build harness dependencies and CMake/Make complexity. |
| **Inline Assembly & Intrinsics** | **Strong**: `core::arch::asm!` is stabilized, typed, and checked against register constraints. MSR, CR0/CR2/CR3/CR4, and segment register access are cleanly encapsulated. | **Strong**: GCC/Clang `__asm__ volatile` syntax is flexible but historically error-prone regarding register clobber constraints. |
| **Concurrency Primitives** | **Superior**: `Send` and `Sync` traits prevent race conditions at compile time across SMP cores and scheduler thread contexts. | **Manual**: Spinlocks and atomic synchronization require rigorous manual auditing without compiler-enforced data isolation. |
| **Debugging & Symbolication** | **Good**: DWARF debug symbols generated natively via LLVM. Demangling required in GDB/LLDB. Zero-overhead abstractions sometimes optimize out local frames in release builds. | **Excellent**: Mature native DWARF and PDB support across all toolchains and system debuggers. |
| **Build Reproducibility** | **High**: `Cargo.lock` pins dependency trees deterministically. Cargo handles cross-compilation targets out-of-the-box (`--target x86_64-unknown-none`). | **Variable**: Highly dependent on specific installed toolchain packages, system headers, and makefile flags across host platforms. |

### 2.2 Architectural Recommendation: Hybrid Layering

1. **Kernel Core & Subsystems (Rust `x86_64-unknown-none`)**:
   - The core kernel, page-table managers, physical memory allocators, scheduler, and IPC subsystems should be written in **Rust** using `#![no_std]` and `#![no_main]`.
   - The memory safety guarantees eliminate an entire class of security vulnerabilities (use-after-free, kernel heap corruption, race conditions in multi-core SMP scheduling).
   - Rust's `extern "win64"` calling convention support allows zero-cost native dispatch for Windows NT-compatible syscalls and user-space thread contexts.

2. **Early Boot & PE Loader / Compatibility Stubs (C & Rust `#[repr(C)]`)**:
   - The PE/COFF parser and Win32 emulation layer utilize explicit `#[repr(C)]` layouts matching Microsoft PE32+ header specifications (`IMAGE_DOS_HEADER`, `IMAGE_NT_HEADERS64`, `IMAGE_SECTION_HEADER`).
   - Low-level assembly trampolines (`switch_to_user`, interrupt vectors) will be maintained in dedicated `.S` assembly files or controlled `asm!` blocks.

---

## 3. Emulation Fixture: QEMU & OVMF Setup

To provide a deterministic, cross-platform testing environment for developers and CI runners, KntonOS targets **QEMU (`qemu-system-x86_64`)** with **OVMF (Open Virtual Machine Firmware / EDK2)**.

### 3.1 Firmware Configuration
- **OVMF Firmware Path**: Prebuilt 64-bit OVMF images (`OVMF_CODE.fd` and `OVMF_VARS.fd`).
- **Read-Only Code / Writable NVRAM**:
  ```bash
  -drive if=pflash,format=raw,readonly=on,file=firmware/OVMF_CODE.fd \
  -drive if=pflash,format=raw,file=target/OVMF_VARS.fd
  ```
  Isolating `OVMF_VARS.fd` into a scratch build directory ensures clean boot state across runs without persistent NVRAM state corruption.

### 3.2 Hardware Profile Specification

The reference virtual machine configuration reflects a modern x86-64 UEFI workstation with low-overhead paravirtualized devices:

```bash
qemu-system-x86_64 \
  -machine q35,smm=on,accel=kvm:whpx:tcg \
  -cpu max,smap=on,smep=on \
  -m 2048M \
  -smp 4,cores=4,threads=1 \
  -drive if=pflash,format=raw,readonly=on,file=firmware/OVMF_CODE.fd \
  -drive if=pflash,format=raw,file=target/OVMF_VARS.fd \
  -drive format=raw,file=target/disk.img,if=none,id=nvme0 \
  -device nvme,serial=knton-boot-0,drive=nvme0 \
  -device virtio-gpu-pci,xres=1280,yres=720 \
  -device virtio-net-pci,netdev=net0 \
  -netdev user,id=net0 \
  -device nec-usb-xhci,id=xhci \
  -device usb-kbd,bus=xhci.0 \
  -device usb-mouse,bus=xhci.0 \
  -serial stdio \
  -display none
```

### 3.3 Hardware Feature Justification

1. **Machine Type (`q35`)**: Provides native PCIe root complex, modern AHCI/NVMe topology, and APIC/IOAPIC interrupt routing matching modern UEFI motherboards.
2. **Acceleration Chain (`accel=kvm:whpx:tcg`)**:
   - `kvm`: Hardware acceleration on Linux development and CI hosts.
   - `whpx`: Windows Hypervisor Platform acceleration on Windows workstations.
   - `tcg`: Portable software emulation fallback for environments lacking nested virtualization.
3. **Security Features (`smap=on,smep=on`)**:
   - **SMEP** (Supervisor Mode Execution Prevention): Prevents kernel from executing user-space memory pages.
   - **SMAP** (Supervisor Mode Access Prevention): Prevents kernel from inadvertent reads/writes to user-space memory without explicit override flags (`stac`/`clac`).
4. **Display & Serial Multiplexing**:
   - Headless execution uses `-display none -serial stdio` or `-nographic`.
   - Low-level debug output is multiplexed through the COM1 UART (16550A) at I/O port `0x3F8`.

---

## 4. Automated CI Test Harness

The automated test runner provides deterministic validation of kernel boot stages without requiring manual GUI observation. It executes QEMU in a headless subprocess, inspects the 16550 serial stream in real time, monitors for sentinel tokens, and enforces hard timeouts.

### 4.1 Sentinel Protocol Specification

The kernel outputs structured sentinel log lines to COM1:
- `[KNT-BOOT] START: Core bootstrap initialized`
- `[KNT-MEM] Paging initialized (CR3: 0x...)`
- `[KNT-GOP] Framebuffer acquired: 1280x720x32bpp`
- `[KNT-TEST] PASS: <subsystem_name>`
- `[KNT-PANIC] <diagnostic_information>`
- `[KNT-BOOT] SUCCESS: System ready`

### 4.2 Cross-Platform Harness Implementation (`tools/ci/boot_runner.py`)

Below is the production-grade runner design for integration into CI workflows:

```python
#!/usr/bin/env python3
"""KntonOS Headless Boot Test Runner.

Executes QEMU with OVMF firmware, monitors serial output for sentinel
tokens, verifies success criteria, and exits with 0 (pass) or 1 (fail).
"""

import argparse
import os
import re
import subprocess
import sys
import time

SENTINEL_SUCCESS = re.compile(r"\[KNT-BOOT\] SUCCESS: System ready")
SENTINEL_PANIC = re.compile(r"\[KNT-PANIC\]")
SENTINEL_PASS = re.compile(r"\[KNT-TEST\] PASS: (.+)")

def run_boot_test(disk_image: str, firmware_dir: str, timeout_sec: int) -> int:
    ovmf_code = os.path.join(firmware_dir, "OVMF_CODE.fd")
    ovmf_vars = os.path.join(firmware_dir, "OVMF_VARS.fd")

    if not os.path.isfile(disk_image):
        print(f"[ERROR] Disk image not found: {disk_image}", file=sys.stderr)
        return 1
    if not os.path.isfile(ovmf_code):
        print(f"[ERROR] OVMF_CODE not found: {ovmf_code}", file=sys.stderr)
        return 1

    cmd = [
        "qemu-system-x86_64",
        "-machine", "q35,accel=tcg",
        "-cpu", "max,smap=on,smep=on",
        "-m", "1024M",
        "-smp", "2",
        "-drive", f"if=pflash,format=raw,readonly=on,file={ovmf_code}",
        "-drive", f"if=pflash,format=raw,file={ovmf_vars}",
        "-drive", f"format=raw,file={disk_image},if=none,id=bootdisk",
        "-device", "nvme,drive=bootdisk,serial=knton-ci",
        "-display", "none",
        "-serial", "stdio",
        "-no-reboot",
    ]

    print(f"[HARNESS] Launching QEMU (timeout: {timeout_sec}s)...")
    start_time = time.time()
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )

    passed_tests = []
    success = False
    panic_detected = False

    try:
        while True:
            # Check elapsed time
            elapsed = time.time() - start_time
            if elapsed > timeout_sec:
                print(f"\n[HARNESS] TIMEOUT exceeded ({timeout_sec}s). Terminating QEMU.", file=sys.stderr)
                break

            line = proc.stdout.readline()
            if not line and proc.poll() is not None:
                break
            if line:
                sys.stdout.write(f"[SERIAL] {line}")
                sys.stdout.flush()

                if SENTINEL_PANIC.search(line):
                    panic_detected = True

                match_test = SENTINEL_PASS.search(line)
                if match_test:
                    passed_tests.append(match_test.group(1))

                if SENTINEL_SUCCESS.search(line):
                    success = True
                    break

    finally:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                proc.kill()

    print("\n--- Boot Test Summary ---")
    print(f"Elapsed Time: {time.time() - start_time:.2f}s")
    print(f"Verified Tests: {len(passed_tests)}")
    for t in passed_tests:
        print(f"  * {t}")

    if panic_detected:
        print("[HARNESS] RESULT: FAILED (Kernel panic detected)", file=sys.stderr)
        return 1
    if not success:
        print("[HARNESS] RESULT: FAILED (Success sentinel not reached)", file=sys.stderr)
        return 1

    print("[HARNESS] RESULT: PASSED")
    return 0

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="KntonOS Boot Test Runner")
    parser.add_argument("--image", required=True, help="Path to bootable raw disk image")
    parser.add_argument("--firmware-dir", required=True, help="Directory containing OVMF_CODE.fd and OVMF_VARS.fd")
    parser.add_argument("--timeout", type=int, default=30, help="Timeout in seconds (default: 30)")
    args = parser.parse_args()
    sys.exit(run_boot_test(args.image, args.firmware_dir, args.timeout))
```

---

## 5. Kernel Debugging & Diagnostic Setup

### 5.1 Remote GDB/LLDB Integration
QEMU provides a built-in GDB stub accessed via `-s -S`:
- `-s`: Shorthand for `-gdb tcp::1234`. Opens a GDB remote connection on port 1234.
- `-S`: Freezes the virtual CPU at startup, allowing breakpoints to be established prior to the execution of the first instruction at the reset vector (`0xFFFFFFF0` in real mode or UEFI entry point).

**Debugger Connection Script (`.gdbinit`)**:
```gdb
target remote :1234
set architecture i386:x86-64
set pagination off

# Load symbols at kernel virtual base offset (e.g. Higher-Half 0xFFFFFFFF80000000)
# add-symbol-file target/x86_64-unknown-none/debug/knton_kernel 0xFFFFFFFF80000000

define knton-stack
  info registers
  x/16gx $rsp
end

define knton-cr
  printf "CR0: 0x%016lx\n", $cr0
  printf "CR2: 0x%016lx\n", $cr2
  printf "CR3: 0x%016lx\n", $cr3
  printf "CR4: 0x%016lx\n", $cr4
end
```

### 5.2 Interrupt Descriptor Table (IDT) & Exception Architecture

A production-grade bare-metal kernel must catch CPU architectural exceptions before they devolve into triple faults and unhandled hardware resets.

```
       +-------------------------------------------------------------+
       |               x86-64 CPU Exception Occurs                   |
       +-------------------------------------------------------------+
                                      |
                      Does Exception specify IST index?
                       /                             \
                     Yes                              No
                     /                                 \
        +---------------------------+     +--------------------------+
        | Switch to Dedicated Stack |     | Push onto Current Stack  |
        | (e.g., IST1 for #DF)      |     | (RSP -= Frame Size)      |
        +---------------------------+     +--------------------------+
                     \                                 /
                      +---------------+---------------+
                                      |
       +-------------------------------------------------------------+
       | Push Exception Frame: SS, RSP, RFLAGS, CS, RIP, [ErrorCode] |
       +-------------------------------------------------------------+
                                      |
       +-------------------------------------------------------------+
       | Save General Registers (RAX, RBX, RCX, RDX, RSI, RDI, etc.)|
       +-------------------------------------------------------------+
                                      |
       +-------------------------------------------------------------+
       | Call Rust Exception Handler (Formatted Serial Output)       |
       +-------------------------------------------------------------+
```

#### Dedicated Interrupt Stack Table (IST) Setup
To survive stack overflow faults, the Task State Segment (TSS) must configure dedicated stacks:
- **IST1 (Double Fault `#DF`, Vector 8)**: Prevents immediate triple-fault reboot when a page fault `#PF` occurs with an exhausted or corrupted kernel stack.
- **IST2 (Non-Maskable Interrupt `#NMI`, Vector 2)**: Critical hardware error diagnostics.
- **IST3 (Machine Check Exception `#MC`, Vector 18)**: Unrecoverable hardware bus errors.

### 5.3 Panic Diagnostics & Register Dump Formatter

When a kernel panic occurs, the early diagnostic subsystem formats state directly to the COM1 UART port (`0x3F8`) without relying on heap allocators or dynamic locks:

```
================================================================================
                         KNTON-OS KERNEL PANIC
================================================================================
Reason: Unhandled Page Fault (#PF) in Kernel Space
Location: src/mm/paging.rs:142
Address Faulted (CR2): 0x0000000000000028
Error Code: 0x0000000000000000 (P=0, W=0, U=0, R=0, I=0)
  * Violation: Page not present
  * Access: Read operation
  * Privilege: Supervisor (Kernel) mode

--- CPU Register Dump ---
RIP: 0x0010:0xFFFFFFFF80104A20   RFLAGS: 0x00010286 [IOPL=0 IF TF NT]
RAX: 0x0000000000000000   RBX: 0xFFFFFFFF80205000   RCX: 0x0000000000000001
RDX: 0x00000000000003F8   RSI: 0xFFFFFFFF8010E040   RDI: 0x0000000000000028
RBP: 0xFFFFFFFF803FFEB0   RSP: 0xFFFFFFFF803FFE98   R8 : 0x0000000000000000
R9 : 0x0000000000000000   R10: 0x0000000000000000   R11: 0x0000000000000246
R12: 0x0000000000000000   R13: 0x0000000000000000   R14: 0x0000000000000000
R15: 0x0000000000000000

--- Control Registers ---
CR0: 0x0000000080010033 (PG, PE, WP, MP, ET)
CR2: 0x0000000000000028 (Faulting Linear Address)
CR3: 0x0000000000201000 (PML4 Base)
CR4: 0x0000000000350EF0 (FSGSBASE, PAE, PGE, OSFXSR, OSXMMEXCPT, SMEP, SMAP)

--- Stack Backtrace ---
#0  0xFFFFFFFF80104A20 in knton_kernel::mm::paging::map_page ()
#1  0xFFFFFFFF80102140 in knton_kernel::kmain ()
#2  0xFFFFFFFF80100080 in _start ()
================================================================================
System halted. Awaiting debugger attachment on TCP port 1234.
```

---

## 6. Actionable Next Steps & Integration Matrix

1. **TASK-KNT-003 Integration**: Provide this analysis to the Platform & Compatibility ADR to confirm Rust (`x86_64-unknown-none`) as the primary kernel development target and establish the headless QEMU fixture as the standard validation platform.
2. **TASK-KNT-004 Synergy**: Ada Chen's performance benchmarking harness can directly ingest the serial test harness and QEMU hardware profile defined herein.
3. **Repository Tooling Bootstrap**: Add `tools/ci/boot_runner.py` and QEMU run configurations to the repository once implementation workspaces open.
