# KntonOS Native Boot Guide & Verification Runbook

- **Task**: TASK-KNT-006 (Native UEFI/QEMU boot proof)
- **Author**: @JulesMartin (Developer, Knton OS)
- **Status**: Implemented & Verified

---

## 1. Overview

This document describes the reproducible build and validation procedures for the first native x86-64 boot boundary in KntonOS. The implementation comprises:
1. `boot/`: The minimal UEFI bootloader (`kboot.efi`), built for `x86_64-unknown-uefi`. It initializes the COM1 serial UART (`0x3F8`), outputs `[KNT-UEFI-START]`, and transfers control to the kernel.
2. `kernel/`: The independent KntonOS micro-kernel (`kntonk`), providing the core entry point (`kmain`), which initializes serial diagnostics, outputs the deterministic sentinel `[KNT-BOOT-SUCCESS]`, and halts the CPU in a low-power `hlt` loop.
3. `scripts/boot_runner.py`: The cross-platform automated test runner that sets up the EFI System Partition (ESP), executes QEMU headlessly with OVMF firmware, monitors serial output, asserts the boot sentinel within a bounded timeout, and returns exit code 0 (pass) or 1 (fail).

---

## 2. Dependencies & Provenance

To guarantee 100% offline reproducibility and total licensing safety:
- **Language & Runtime**: Rust `stable` (edition 2021), target `x86_64-unknown-uefi` and `x86_64-unknown-none`.
- **Third-Party Crates**: **None** (zero external dependencies). The implementation relies solely on the standard Rust `core` library (`#![no_std]`).
- **Firmware**: Open Virtual Machine Firmware (OVMF / EDK2), `edk2-x86_64-code.fd` (BSD-2-Clause-Patent).
- **Emulation**: QEMU (`qemu-system-x86_64`, GPLv2).
- **Licensing Posture**: All custom code is proprietary (All Rights Reserved / ARR) under the KntonOS project.

---

## 3. Build Instructions

From the workspace root (`kntonOS-jules-martin`):

```bash
# 1. Ensure the UEFI target is installed
rustup target add x86_64-unknown-uefi x86_64-unknown-none

# 2. Build the UEFI bootloader and kernel
cargo build --package kboot --target x86_64-unknown-uefi
```

The resulting UEFI PE32+ executable is located at:
`target/x86_64-unknown-uefi/debug/kboot.efi`

---

## 4. Automated Testing & Verification

Run the automated headless test runner:

```bash
python scripts/boot_runner.py
```

### Execution Flow:
1. Detects `qemu-system-x86_64` and `edk2-x86_64-code.fd`.
2. Populates `target/esp/EFI/BOOT/BOOTX64.EFI`.
3. Launches QEMU with `-display none -serial stdio -drive if=pflash,... -drive file=fat:rw:target/esp...`.
4. Asserts deterministic serial output:
   - `[KNT-UEFI-START] KntonOS UEFI Bootloader (kboot.efi) initialized`
   - `[KNT-KERNEL-INIT] KntonOS micro-kernel bootstrap reached`
   - `[KNT-BOOT-SUCCESS] Native UEFI-to-kernel boot verified`
5. Terminates QEMU cleanly and returns exit code `0`.
