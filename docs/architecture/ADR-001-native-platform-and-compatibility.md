# ADR-001: Native Platform Architecture and Windows Application Compatibility Boundary

- **Status**: Proposed / In Review
- **Date**: 2026-10-29
- **Authors**: Marcus (Senior Dev / Implementation Lead)
- **Task Reference**: TASK-KNT-003
- **Reviewers**: Nadia Mercer (Project Maintainer / Independent Lead Review)
- **Scope**: Native x86-64 UEFI Kernel, Core User-space Architecture, and Windows PE/.EXE Compatibility Model

---

## 1. Context and Problem Statement

The owner has defined KntonOS as an independently developed operating system targeting modern x86-64 UEFI hardware, featuring:
1. An independent, bespoke kernel (no copied or modified third-party kernels such as Linux, XNU, or BSD).
2. A lightweight, modern GUI environment delivering a Windows 11-like desktop feel with dramatically lower resource consumption (RAM, storage, background CPU cycles).
3. The strategic capability to run Windows `.exe` applications, specifically focusing on games and everyday productivity software.

This architecture decision record (ADR) analyzes the native platform design, kernel/user-space separation, component language selections, and a staged compatibility boundary to achieve a credible first vertical slice without making unbounded claims.

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
  - Memory Management: Physical page allocation, virtual address space management (PML4 paging), user-space page fault handlers, copy-on-write mappings.
  - Preemption & Threading: SMP-aware round-robin with dynamic priority boosts, cooperative yield, thread contexts (`RSP`, `RIP`, registers, SSE/AVX state saving via `FXSAVE`/`XSAVE`).
  - Synchronization: Spinlocks (irq-safe, kernel internal), Mutexes, Semaphores, Event objects.
  - IPC: Fast synchronous and asynchronous message-passing ports utilizing shared-memory ring buffers and futex-like wake primitives.
  - Driver Interface: Modular kernel driver interface for critical low-latency hardware (Timers, LAPIC/IOAPIC, NVMe/AHCI storage, Framebuffer/GPU command submission).
- **User Space (Ring 3)**:
  - File System Daemons: Modular VFS servers running as isolated processes.
  - Graphics Compositor: Hardware-accelerated desktop compositing engine managing surface windows, input events, and rendering.
  - Desktop Shell: UI framework rendering taskbar, start menu, system tray, and window decorations.
  - Windows Compatibility Subsystem (KWin Subsystem): User-mode translation servers, PE loader, and Win32/DirectX bridging runtimes.

---

## 3. Implementation Language Evaluation by Component

| Component | Evaluated Languages | Evaluation & Trade-offs | Decision |
| :--- | :--- | :--- | :--- |
| **Kernel Core & Bootloader** | **Rust vs. C (C23) vs. Modern C++** | - **C (C23)**: Ubiquitous, simple ABI, zero compiler abstractions, but lacks memory safety; vulnerable to buffer overflows and use-after-free.<br>- **Modern C++**: Heavy runtime requirements, exception tables, complex bare-metal subsetting.<br>- **Rust (`no_std`, `x86_64-unknown-none`)**: Memory safety without GC, type-level ownership preventing data races, robust `uefi` and `x86_64` bare-metal ecosystem, zero-cost abstractions, excellent tooling (`cargo`, `clippy`). | **Rust (`no_std`) for Kernel & Bootloader Core**. Safe abstractions wrap necessary `unsafe` hardware accesses (port I/O, MMIO, paging). |
| **Core Drivers (Storage/Network)** | **Rust vs. C** | Rust provides memory-safe packet and block parsing, preventing catastrophic driver crashes. Clean C-FFI permits C-based driver interfaces where needed. | **Rust** (with optional C-compatible ABI boundaries). |
| **Compositor & GUI Framework** | **Rust vs. C++ / C** | Fast rendering requires tight memory control and concurrency. Rust's memory safety prevents UI crashes; modern UI crates (or a custom Vulkan/software-rasterized widget pipeline) can be cleanly built with strong guarantees. | **Rust** for native shell, compositor, and system applications. |
| **Windows Compatibility Runtime** | **Rust + C (MSVC ABI)** | Interfacing with Windows PE binaries requires strict adherence to the Microsoft x64 calling convention (`rcx, rdx, r8, r9`, shadow space), PE32+ header parsing, and SEH (Structured Exception Handling). Rust supports `extern "win64"` natively, allowing seamless interop. | **Rust** with targeted Win64 FFI shims. |

---

## 4. Windows `.exe` and Game Compatibility Strategy

### 4.1 Fundamental Architecture of the Compatibility Subsystem

Running Windows `.exe` binaries on an independent native kernel requires solving four distinct layers:

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
   - Direct translation of D3D commands into Vulkan shaders/command buffers (leveraging open translation architectures akin to DXVK/VKD3D) allows native GPU drivers to execute game draw calls without running Windows kernel graphics drivers.
4. **Legal & Intellectual Property Boundaries**:
   - **Zero Proprietary Code**: No Windows binaries, leaked Microsoft source code, or proprietary DLLs will be incorporated.
   - **Clean-room Interface Specifications**: Reimplementation will be strictly based on publicly documented Microsoft Win32/PE specifications and open API definitions.

### 4.2 Staged Compatibility Plan

- **Stage 1 (Vertical Slice Proof-of-Concept)**:
  - Run a 64-bit console `.exe` (e.g., standard C runtime "Hello World" or computational benchmark) compiled with MSVC/MinGW.
  - Scope: PE loader, basic `ntdll`/`kernel32` shims (`ExitProcess`, `WriteFile` to stdout, `GetStdHandle`, `VirtualAlloc`).
- **Stage 2 (Graphical Win32 Application)**:
  - Run a standard 2D Win32 GUI application (e.g., Notepad-style utility or simple GDI demo).
  - Scope: Basic `user32` event loop (`GetMessage`, `DispatchMessage`, window creation, mouse/keyboard input mapped to the Knton compositor).
- **Stage 3 (Hardware-Accelerated 3D / Real Game Demo)**:
  - Run a targeted lightweight 3D game (e.g., a real open-source/indie game target or benchmark using DX9/DX11/Vulkan).
  - Scope: XInput controller support, DirectSound/WASAPI audio output, GPU command queue integration via Vulkan.

---

## 5. First Credible Native Vertical Slice

To prove the core value proposition without premature overextension, the first milestone must demonstrate:
1. **Bare-metal UEFI Boot**: `kboot.efi` loads `kntonk` on an x86-64 machine (or QEMU/OVMF) without any host OS.
2. **Native Kernel Ring 0 Setup**: Page tables active, preemptive scheduler switching between multiple kernel worker threads, timer interrupts firing.
3. **Transition to Ring 3**: First user-space process spawned with clean memory protection.
4. **Interactive GUI Compositor**: Framebuffer initialized, displaying a clean, responsive desktop panel and window frame (Windows 11 inspired).
5. **Execution of First Windows .EXE**: The PE loader parses and executes a self-contained 64-bit Windows executable, rendering its output on screen.

---

## 6. Validation Fixtures and Testing Strategy

- **Emulation Tier (CI/CD Automated)**:
  - QEMU (`qemu-system-x86_64`) with OVMF UEFI firmware, executing headlessly in automated test passes.
  - Serial port (COM1) telemetry capturing assertions, benchmark metrics, and boot checkpoints.
- **Hardware Tier (Acceptance Verification)**:
  - Dedicated reference physical x86-64 test machines (UEFI mode, NVMe/USB boot).
  - Power and memory measurement fixtures for validating idle consumption.

---

## 7. Open Decisions for Owner Review

1. **First-Demo Benchmark Game Selection**: Confirmation of the specific title or benchmark to target for the first public demonstration (detailed in SPEC-001 by Ada Chen).
2. **GPU Driver Integration Pathway**: Confirmation whether native GPU driver development will prioritize Intel/AMD open command submission specs or a universal Vulkan bare-metal driver model.

---

## 8. Summary of Recommendations

1. Adopt a **Hybrid Modular Kernel** in **Rust (`no_std`)** for maximum safety, performance, and fault isolation.
2. Retain **UEFI long-mode boot** via a custom boot executive (`kboot.efi`).
3. Implement the **Windows compatibility boundary entirely in Ring 3** via a clean-room PE loader and user-space API shims.
4. Target a **staged 3-phase compatibility rollout**, proving console PE execution first before advancing to complex 3D graphics pipelines.
