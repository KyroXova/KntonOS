# ADR-001: Native Platform Architecture and Windows Application Compatibility Boundary

- **Status**: Proposed / Revised (Revision 2)
- **Date**: 2026-10-29
- **Authors**: Marcus (Senior Dev / Implementation Lead)
- **Task Reference**: TASK-KNT-003
- **Reviewers**: Nadia Mercer (Project Maintainer / Independent Lead Review)
- **Scope**: Native x86-64 UEFI Kernel, Core User-space Architecture, GPU Display Pipeline, and Windows PE/.EXE Compatibility Model

---

## 1. Context and Problem Statement

The owner has defined KntonOS as an independently developed operating system targeting modern x86-64 UEFI hardware, featuring:
1. An independent, bespoke kernel (no copied or modified third-party kernels such as Linux, XNU, or BSD).
2. A lightweight, modern GUI environment delivering a Windows 11-like desktop feel with dramatically lower resource consumption (RAM, storage, background CPU cycles).
3. The strategic capability to run Windows `.exe` applications, specifically focusing on games and everyday productivity software.

This architecture decision record (ADR) analyzes the native platform design, kernel/user-space separation, component language selections, explicit GPU driver dependencies, dependency licensing provenance, and a staged compatibility boundary to achieve a credible first vertical slice without making unbounded claims.

---

## 2. Kernel and System Architecture

### 2.1 Kernel Paradigm Evaluation

| Paradigm | Description | Pros for KntonOS | Cons / Risks | Recommendation |
| :--- | :--- | :--- | :--- | :--- |
| **Monolithic** | All OS services (drivers, memory, scheduler, filesystems, networking) run in Ring 0. | Highest raw performance, simple call conventions, lower context-switch overhead for graphics/games. | High crash surface; driver bugs cause blue screens / panics; difficult to sandbox compatibility layers. | **Not Recommended** for native design. |
| **Pure Microkernel** | Only IPC, scheduling, and minimal memory primitives in Ring 0. Drivers and FS run in Ring 3. | Maximum isolation and fault resilience; crash in a driver or compatibility service does not kill the machine. | Significant IPC overhead, frame-time latency jitter for high-throughput graphics / input pipelines unless heavily optimized. | Viable, but IPC optimization cost is extreme for games. |
| **Hybrid / Modular Kernel** | Small core executive with capability-based memory management, preemptive scheduling, and Ring 0 graphics/memory acceleration paths, while subsystems (Win32 compatibility subsystem, filesystem servers, networking, user shell) execute in isolated Ring 3 domains. | Optimal balance: protected Ring 0 core, low-overhead direct rendering pathways, isolated crash-safe compatibility runtimes. Matches NT/XNU organizational elegance without replicating NT proprietary internals. | Requires disciplined capability and interface boundary design from day one. | **RECOMMENDED** |

### 2.2 Boot-to-User-Space Sequence (x86-64 UEFI)

```
[UEFI Firmware] 
       │ (x86-64 long mode, UEFI GOP, memory map, ACPI tables)
       ▼
[Knton Boot Executive (kboot.efi)]
       │ - Allocates physical pages for kernel & initial ramdisk
       │ - Sets up identity paging & higher-half kernel virtual address space
       │ - Acquires UEFI GOP framebuffer pointers & ACPI RSDP
       │ - Exits UEFI Boot Services (ExitBootServices)
       ▼
[KntonOS Kernel Core (Ring 0: kntonk)]
       │ - Initializes GDT, IDT, TSS, ISRs, early serial output (COM1/0x3F8)
       │ - Initializes Physical Memory Manager (Bitmap/Buddy Allocator)
       │ - Sets up Virtual Memory Manager (PML4 4-level paging, recursive/direct mapping)
       │ - Probes ACPI / LAPIC / IOAPIC / HPET timers
       │ - Initializes Thread Scheduler (preemptive priority-queue scheduler)
       │ - Initializes IPC & Capability subsystem
       ▼
[Init / Service Manager (Ring 3: kinit)]
       │ - Spawns Device Manager & Storage Driver (VFS / NVMe / AHCI)
       │ - Spawns Native Display & Compositor Server (kcompositor)
       │ - Launches User Shell & Desktop Environment (kshell)
       ▼
[Interactive Windows 11-like Desktop Shell]
```

### 2.3 Kernel / User-space Responsibility Boundary

- **Kernel Core (Ring 0)**:
  - **Memory Management**: Physical page allocation, virtual address space management (PML4 paging), user-space page fault handlers, copy-on-write mappings.
  - **Preemption & Threading**: SMP-aware round-robin with dynamic priority boosts, cooperative yield, thread contexts (`RSP`, `RIP`, registers, SSE/AVX state saving via `FXSAVE`/`XSAVE`).
  - **Synchronization**: Spinlocks (irq-safe, kernel internal), Mutexes, Semaphores, Event objects.
  - **IPC**: Fast synchronous and asynchronous message-passing ports utilizing shared-memory ring buffers and futex-like wake primitives.
  - **Driver Interface**: Modular kernel driver interface for critical low-latency hardware (Timers, LAPIC/IOAPIC, NVMe/AHCI storage, GPU command submission).
- **User Space (Ring 3)**:
  - **File System Daemons**: Modular VFS servers running as isolated processes.
  - **Graphics Compositor**: Hardware-accelerated desktop compositing engine managing surface windows, input events, and rendering.
  - **Desktop Shell**: UI framework rendering taskbar, start menu, system tray, and window decorations.
  - **Windows Compatibility Subsystem (KWin Subsystem)**: User-mode translation servers, PE loader, and Win32/DirectX bridging runtimes.

---

## 3. Implementation Language Evaluation by Component

| Component | Evaluated Languages | Evaluation & Trade-offs | Decision |
| :--- | :--- | :--- | :--- |
| **Kernel Core & Bootloader** | **Rust vs. C (C23) vs. Modern C++** | - **C (C23)**: Ubiquitous, simple ABI, zero compiler abstractions, but lacks memory safety; vulnerable to buffer overflows and use-after-free.<br>- **Modern C++**: Heavy runtime requirements, exception tables, complex bare-metal subsetting.<br>- **Rust (`no_std`, `x86_64-unknown-none`)**: Memory safety without GC, type-level ownership preventing data races, robust `uefi` and `x86_64` bare-metal ecosystem, zero-cost abstractions, excellent tooling (`cargo`, `clippy`). | **Rust (`no_std`) for Kernel & Bootloader Core**. Safe abstractions wrap necessary `unsafe` hardware accesses (port I/O, MMIO, paging). |
| **Core Drivers (Storage/Network)** | **Rust vs. C** | Rust provides memory-safe packet and block parsing, preventing catastrophic driver crashes. Clean C-FFI permits C-based driver interfaces where needed. | **Rust** (with optional C-compatible ABI boundaries). |
| **Compositor & GUI Framework** | **Rust vs. C++ / C** | Fast rendering requires tight memory control and concurrency. Rust's memory safety prevents UI crashes; modern UI crates (or a custom Vulkan/software-rasterized widget pipeline) can be cleanly built with strong guarantees. | **Rust** for native shell, compositor, and system applications. |
| **Windows Compatibility Runtime** | **Rust + C (MSVC ABI)** | Interfacing with Windows PE binaries requires strict adherence to the Microsoft x64 calling convention (`rcx, rdx, r8, r9`, shadow space), PE32+ header parsing, and SEH (Structured Exception Handling). Rust supports `extern "win64"` natively, allowing seamless interop. | **Rust** with targeted Win64 FFI shims. |

---

## 4. Display Pipeline and Explicit GPU Driver Dependencies

A critical architectural distinction exists between **2D desktop rendering** and **3D hardware-accelerated game execution**:

```
┌────────────────────────────────────────────────────────────────────────┐
│                        2D Desktop Environment                          │
│         (Windows 11-style Desktop Shell, Taskbar, Menus, Windows)      │
└────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│             Linear Framebuffer Pipeline (UEFI GOP / Direct)            │
│  - Zero GPU vendor driver dependency                                   │
│  - Fully functional on all UEFI 2.x x86-64 machines out of the box     │
│  - Fast CPU SIMD/AVX-accelerated 2D blitting & double-buffering        │
└────────────────────────────────────────────────────────────────────────┘

──────────────────────────────────────────────────────────────────────────

┌────────────────────────────────────────────────────────────────────────┐
│                    Windows 3D Games & Applications                     │
│                  (.EXE, Direct3D 9/11/12, Vulkan, OpenGL)              │
└────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│             Hardware-Accelerated GPU Driver Stack (MANDATORY)          │
│  - Hardware-specific kernel GPU drivers (AMD RADV/amdgpu, Intel ANV)   │
│  - Virtualized GPU drivers for CI/emulation (VirtIO-GPU)               │
│  - DirectX-to-Vulkan translation runtime (DXVK / VKD3D architecture)   │
│  - Direct hardware command-ring submission & VRAM page management      │
└────────────────────────────────────────────────────────────────────────┘
```

### 4.1 2D Desktop Pipeline (Zero Vendor GPU Driver Dependency)
1. **UEFI GOP Handover**: During boot, `kboot.efi` queries the UEFI Graphics Output Protocol (GOP) for physical framebuffer base address, resolution, pixel format, and stride.
2. **Kernel Framebuffer Driver**: The kernel retains this linear framebuffer, mapping it as Write-Combining (`PAT`/`MTRR`) memory.
3. **Software Compositor**: The desktop compositor renders windows, taskbars, and animations using SIMD/AVX2 blitting directly into a double-buffered memory region before flipping to the GOP buffer.
4. **Architectural Benefit**: Allows KntonOS to boot to a fully functional, responsive, beautiful Windows 11-like desktop on **any modern UEFI x86-64 PC** without needing proprietary NVIDIA/AMD/Intel drivers.

### 4.2 3D Game Acceleration Pipeline (Explicit Hardware GPU Driver Dependency)
1. **Hard Dependency**: 3D games and DirectX/Vulkan applications **cannot execute on a linear GOP framebuffer**. They require direct access to GPU compute engines, rasterizers, and VRAM.
2. **Driver Staging Strategy**:
   - **Virtual Target (CI / First Proof)**: VirtIO-GPU driver in KntonOS kernel, enabling automated 3D testing inside QEMU.
   - **Physical Target (Demo Tier)**: Open command submission driver targeting AMD Radeon (GCN/RDNA via open AMDGPU spec) and Intel Iris Xe/Arc (via open Intel Xe/i915 spec), avoiding closed proprietary binary blobs.
   - **Vulkan Interface**: The native graphics driver presents a standard Vulkan 1.3 entry-point.
3. **DirectX-to-Vulkan Translation**: Win32 DirectX calls are translated in user space directly into Vulkan command buffers, allowing Windows games to render at native speeds.

---

## 5. Staged Native Boot Slices

To avoid an unbounded implementation scope, native development is divided into discrete, verifiable slices:

| Slice | Name | Goal & Capability | Validation Criterion | Driver Dependency |
| :--- | :--- | :--- | :--- | :--- |
| **Slice 0** | **UEFI Boot Fixture** | Minimal `kboot.efi` executing under UEFI/OVMF. Initializes COM1 serial port at 115200 baud, reads memory map, logs sentinel `[KNT-BOOT-OK]`, exits boot services cleanly. | Headless QEMU script exits 0 on serial sentinel within 3 seconds. | Firmware GOP only |
| **Slice 1** | **Ring 0 Kernel Core** | `kntonk` sets up 4-level PML4 paging, GDT, IDT (with dedicated `#DF` stack), physical frame allocator, and APIC timer interrupts. | Serial logs demonstrate dynamic memory allocation and preemption. | None |
| **Slice 2** | **Ring 3 Transition** | Kernel executes `sysret` / `iretq` into a user-space task with separate page tables and non-privileged Ring 3 execution rights. | User process triggers a syscall and receives response without Ring 0 fault. | None |
| **Slice 3** | **Interactive 2D Desktop** | Native compositor initializes GOP framebuffer and renders Windows 11-style taskbar, start menu, and draggable window with mouse input. | Visual verification in QEMU and bare-metal PC; idle RAM $\le 300\text{ MB}$. | Linear GOP Framebuffer (Zero vendor drivers) |
| **Slice 4** | **PE/COFF Console Loader** | User-space PE loader parses 64-bit Windows console executable, binds minimal `ntdll`/`kernel32` shims, and executes entry point. | Headless Windows `.exe` executes and writes output to stdout/serial. | None |
| **Slice 5** | **GPU 3D Gaming Demonstration** | GPU driver initializes 3D pipeline; user-space DirectX/Vulkan translation runtime executes a real game binary. | *SuperTuxKart* / *vkQuake* renders and runs at $\ge 100\%$ Windows 11 baseline FPS. | **Explicit Hardware GPU Driver (AMD/Intel/VirtIO-GPU)** |

---

## 6. Dependency and License Provenance

KntonOS adheres to strict intellectual property and license cleanliness standards:

### 6.1 Monorepo Licensing
- **Repository Posture**: The KntonOS repository is **All Rights Reserved (ARR)** by Snatch.
- **Selective Open Source**: Specific public API headers, client SDKs, or ABI definitions may be released under **MIT** only when explicitly approved by the project owner.

### 6.2 Clean-Room Windows Compatibility
- **Zero Proprietary Code**: No Windows binaries, disassembled proprietary binaries, leaked Microsoft source code, or proprietary headers are permitted.
- **Zero Viral Copyleft in Kernel/Core**: Wine and ReactOS code contains LGPL and GPL licenses. To protect KntonOS's intellectual property, **no Wine or ReactOS code may be copied into the kernel or proprietary core services**.
- **Specification-Based Implementation**: PE loader, TEB/PEB layout, and Win32 shims must be implemented cleanly based on:
  - Official Microsoft Open Specifications (e.g., *Microsoft Portable Executable and Common Object File Format Specification*).
  - Open, permissively licensed crates and headers (e.g., Microsoft's official `windows-sys` and `windows-rs` published under MIT/Apache-2.0).

### 6.3 Third-Party Dependency Audit
All external crates included in bare-metal and core packages must strictly hold permissive licenses (MIT, Apache-2.0, or 3-Clause BSD). Every dependency must be pinned with exact versions and verified checksums:
- `x86_64` (MIT / Apache-2.0)
- `uefi` / `uefi-services` (Mozilla Public License 2.0 / Apache-2.0)
- `bootloader_api` (MIT / Apache-2.0)
- `spin` (MIT)
- `volatile` (MIT / Apache-2.0)
- `bitflags` (MIT / Apache-2.0)

---

## 7. Windows `.exe` and Game Compatibility Strategy

```
┌────────────────────────────────────────────────────────┐
│               Windows .EXE / Game Binary               │
└────────────────────────────────────────────────────────┘
                           │ Calls Win32 / DirectX APIs
                           ▼
┌────────────────────────────────────────────────────────┐
│     KWin User-Space Emulation Libraries (DLLs)         │
│  - ntdll.dll, kernel32.dll, user32.dll, gdi32.dll      │
│  - PE Relocations, Import Address Table (IAT) Binding  │
│  - Thread Local Storage (TLS), SEH / Exception Dispatch│
└────────────────────────────────────────────────────────┘
                           │ 
       ┌───────────────────┴───────────────────┐
       ▼                                       ▼
┌──────────────────────────────┐   ┌──────────────────────────────┐
│ Graphics & Multimedia Bridge │   │ System Call / POSIX / Native │
│ - DXVK / D3D to Vulkan bridge│   │ - File I/O (mapped to VFS)   │
│ - DirectInput / XInput bridge│   │ - Memory allocation (Virtual)│
│ - Audio (WASAPI to native)   │   │ - Thread / Sync primitives   │
└──────────────────────────────┘   └──────────────────────────────┘
                           │
                           ▼
┌────────────────────────────────────────────────────────┐
│        Native KntonOS Kernel & Compositor Core         │
└────────────────────────────────────────────────────────┘
```

1. **PE/COFF Loader**:
   - Parses DOS and PE32+ (64-bit) headers.
   - Maps sections (`.text`, `.data`, `.rdata`, `.bss`) into process address space with appropriate page protections (`PAGE_EXECUTE_READ`, `PAGE_READWRITE`).
   - Resolves Import Address Table (IAT) pointers to export symbols provided by KntonOS compatibility DLLs.
   - Sets up initial TEB (Thread Environment Block) and PEB (Process Environment Block) structures pointed to by the `GS` segment register.
2. **Win32 User-Space Subsystem**:
   - Implements Win32 core subsystems entirely in Ring 3.
   - System calls from Windows applications do not directly enter the KntonOS kernel; instead, Windows DLLs call user-space translation wrappers that invoke native KntonOS IPC/syscall interfaces.
3. **Graphics and Shader Execution**:
   - Games rely heavily on DirectX (D3D9, D3D11, D3D12) or Vulkan.
   - Direct translation of D3D commands into Vulkan shaders/command buffers allows native GPU drivers to execute game draw calls without running Windows kernel graphics drivers.

---

## 8. Open Decisions for Owner Review

1. **First-Demo Benchmark Game Selection**: Confirmation of the specific title or benchmark to target for the first public demonstration (detailed in SPEC-001 by Ada Chen).
2. **GPU Driver Development Priority**: Confirmation whether physical GPU driver implementation will target AMD Radeon (GCN/RDNA) first due to publicly available open documentation.

---

## 9. Summary of Recommendations

1. Adopt a **Hybrid Modular Kernel** in **Rust (`no_std`)** for maximum safety, performance, and fault isolation.
2. Retain **UEFI long-mode boot** via a custom boot executive (`kboot.efi`).
3. Build the **2D Windows 11-like desktop on linear GOP framebuffer first**, deferring complex GPU hardware drivers to the 3D gaming slice.
4. Enforce an **explicit GPU driver dependency for 3D gaming**, isolating it from early boot and 2D UI stability.
5. Strictly enforce **clean-room provenance** and permissive licensing for all kernel and Win32 compatibility code.
