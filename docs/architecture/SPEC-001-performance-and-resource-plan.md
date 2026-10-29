# SPEC-001 — First-Demo Resource and Performance Acceptance Plan

| Field | Value |
| --- | --- |
| Document ID | SPEC-001 |
| Task Association | TASK-KNT-004 (First-demo resource and performance acceptance plan) |
| Architecture Tier | Quality Assurance, Benchmarking & Acceptance Criteria |
| Status | Revised Proposal (Source-Verified & Staged Baseline) |
| Author | Ada Chen (@AdaChen) |
| Reviewer | Marcus (@Marcus, Senior Lead) / Nadia Mercer (@NadiaMercer, Project Maintainer) |
| Target Milestone | Staged First Demonstrable KntonOS Vertical Slice (Native Boot -> Desktop -> Real Game) |

---

## 1. Executive Summary & Objective

The strategic charter of KntonOS is to provide an independently developed, lightweight operating system delivering a modern Windows 11-like desktop experience with significantly reduced resource consumption and superior gaming performance for Windows `.exe` applications.

To validate these claims quantitatively with rigorous engineering provenance, this specification establishes:
1. **Source-verified benchmark candidates**, including exact upstream repositories, official release binaries, licensing provenance, graphics rendering APIs, and authoritative command-line syntax.
2. **Reference hardware profiles** and baseline Windows 11 comparison configurations.
3. **Provisional resource and performance budgets** across boot time, idle RAM, ISO size, FPS, and frame-time pacing percentiles.
4. **Staged performance gates**, explicitly decoupling early kernel boot / idle resource acceptance from the real-game performance gate, which carries a hard dependency on native/passthrough GPU driver bring-up.
5. **Repeatable benchmarking protocols**, variance-control procedures, and structured artifact-logging standards.
6. **Clear governance distinction** separating owner-approved strategic targets from provisional engineering budgets.

---

## 2. Governance: Owner Directives vs. Provisional Engineering Budgets

Per `docs/project-scope.md` and repository guidelines, engineering assumptions must never be disguised as owner requirements.

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                       OWNER DIRECTIVES (Strategic Mandate)                      │
│ - Custom kernel, custom GUI, and core applications                             │
│ - Exceptionally lightweight desktop with Windows 11-like feel                   │
│ - Broad Windows .exe compatibility target, prioritizing games                   │
│ - First public demo: Boot KntonOS and run a real game with improved performance │
│ - Licensing: All Rights Reserved (ARR) with selective MIT components            │
└─────────────────────────────────────────────────────────────────────────────────┘
                                       │
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────────┐
│                  PROVISIONAL ENGINEERING BUDGETS (This Document)                │
│ - ISO footprint: <= 500 MB base installation image (Stage 1)                    │
│ - Cold boot time: <= 10.0 seconds from UEFI handoff to desktop idle (Stage 1)   │
│ - Idle OS memory: <= 300 MB resident memory excluding disk cache (Stage 1)      │
│ - Stage 3 Game FPS: >= 100% (target +5% to +15%) of Win11 on identical hardware │
│ - Stage 3 Frame Pacing: 1% low FPS >= 90% of mean; 0.1% low FPS >= 75% of mean  │
└─────────────────────────────────────────────────────────────────────────────────┘
```

- **Owner-Confirmed Directives**:
  - General-purpose OS with preference for gamers and everyday users.
  - Independently developed native x86-64 UEFI kernel (no modified Linux/BSD/NT kernel).
  - Windows `.exe` game execution on bare metal.
  - Demonstrated, measurable performance improvement over Windows.
- **Provisional Engineering Budgets**:
  - Numerical metrics specified in Section 5 are engineering proposals establishing concrete, falsifiable pass/fail gates. They remain provisional until validated against physical hardware benchmarks and formally signed off by the project owner.

---

## 3. Workload Selection: Source-Verified Benchmark Candidates

To eliminate ambiguity and prevent conflation of incompatible graphics stacks, all benchmark candidates are cataloged with their exact upstream repositories, official Windows release packages, software licenses, underlying graphics APIs, and verified execution commands.

### 3.1 Primary 3D Game Candidate: *SuperTuxKart* (Windows x86-64 Portable)
- **Application & Version**: SuperTuxKart v1.4 (Official 64-bit Windows Release).
- **Upstream Source Repository**:
  - Engine & Game Code: [`https://github.com/supertuxkart/stk-code`](https://github.com/supertuxkart/stk-code) (Tag `v1.4`, Commit `e5e493a`)
  - Asset Package: [`https://github.com/supertuxkart/stk-assets`](https://github.com/supertuxkart/stk-assets)
- **Official Binary Artifact**: `SuperTuxKart-1.4-win64.zip` (SourceForge / GitHub Releases, SHA256 verified).
- **Licensing Provenance**:
  - Game Engine & Code: GNU General Public License v3.0 or later (GPLv3+).
  - Game Assets (Models, Audio, Textures): CC-BY-SA 3.0, CC-BY-SA 4.0, CC-BY 3.0, and Public Domain.
- **Rendering Engine & Graphics API**:
  - Custom *Antarctica* rendering engine (forked from Irrlicht 3D).
  - Primary API: **OpenGL 3.3 Core Profile** initialized via Windows native `opengl32.dll` and standard WGL context extensions (`wglCreateContextAttribsARB`).
  - *Note on Graphics Architecture*: STK on Windows executes via `opengl32.dll`. Under KntonOS Stage 3, this executes either via a clean-room `opengl32.dll` shim dispatching to an underlying Vulkan ICD through an OpenGL-on-Vulkan translation layer (e.g. Zink) or via a native OpenGL driver. (Previous draft incorrectly referenced macOS MoltenVK; this draft strictly targets x86-64 Win32/OpenGL32 APIs).
- **Verified Benchmark Execution Command**:
  ```bat
  supertuxkart.exe --profile-time=120 --track=volcano_island --num-karts=8 --mode=0
  ```
  *(Runs an automated 120-second non-interactive race profile on `volcano_island` with 8 AI karts, exercising physics, audio, particle simulation, and multi-pass forward/deferred geometry rendering).*

### 3.2 Secondary Pure-Vulkan Game Candidate: *vkQuake* (Windows x86-64 PE)
- **Application & Version**: vkQuake v1.31.1 (64-bit Windows Release).
- **Upstream Source Repository**: [`https://github.com/Novum/vkQuake`](https://github.com/Novum/vkQuake) (Tag `1.31.1`, Axel Gneiting / Novum).
- **Official Binary Artifact**: `vkQuake-1.31.1-win64.zip` (GitHub Releases).
- **Licensing Provenance**:
  - Engine Code: GNU General Public License v2.0 (GPLv2).
  - Game Data: Id Software Quake Shareware `pak0.pak` (freely redistributable non-commercial shareware).
- **Rendering Engine & Graphics API**:
  - Pure **Vulkan 1.0 / 1.1** renderer directly calling `vulkan-1.dll`.
  - Window Management & Input: Win32 subsystem (`user32.dll`, `kernel32.dll`).
  - *Architectural Significance*: vkQuake provides a zero-wrapper, native Vulkan pipeline. It isolates Win32 PE loader binding, memory mapping, and Vulkan ICD command ring dispatch from complex DirectX translation layers.
- **Verified Benchmark Execution Command**:
  ```bat
  vkquake.exe +host_maxfps 0 +timedemo demo1
  ```
  *(Launches the engine with unrestricted frame rate presentation and immediately executes the standardized 60-second `demo1` timedemo loop, reporting total rendered frames, elapsed time, and average FPS upon completion).*

### 3.3 Diagnostic / Baseline Graphics Suite: *Sascha Willems Vulkan Suite*
- **Application & Version**: Sascha Willems Vulkan Examples (x64 Windows Release, Git master / stable tag).
- **Upstream Source Repository**: [`https://github.com/SaschaWillems/Vulkan`](https://github.com/SaschaWillems/Vulkan).
- **Binary Artifact**: Precompiled 64-bit Windows binaries (`gears.exe`, `deferred.exe`).
- **Licensing Provenance**: MIT License.
- **Rendering Engine & Graphics API**: Native **Vulkan 1.0+** (`vulkan-1.dll`).
- **Verified Benchmark Execution Command**:
  ```bat
  gears.exe -b -f -br 30 -bf "benchmark_gears.csv" -bt
  ```
  *(Executes `gears.exe` in automated benchmark mode (`-b`), fullscreen (`-f`), running for exactly 30 seconds (`-br 30`), exporting individual frame times (`-bt`) to `benchmark_gears.csv` (`-bf`)).*

---

## 4. Hardware Profiles & Windows Comparison Baseline

Performance superiority must be evaluated on identical physical silicon under controlled operating conditions.

### 4.1 Target Hardware Profiles

| Component | Profile A: Reference Dedicated GPU Rig | Profile B: Reference APU / Mobile Rig | Profile C: Virtualized / CI Fixture |
| :--- | :--- | :--- | :--- |
| **Role** | Primary first-demo target | Efficiency / low-end target | Automated regression testing |
| **CPU** | AMD Ryzen 5 5600X (6C/12T, 3.7–4.6 GHz) or Intel Core i5-12400 | AMD Ryzen 5 5600G / 6800U (Radeon 680M) or Intel Iris Xe | QEMU / KVM x86-64 (8 vCPUs, Host Passthrough) |
| **System RAM** | 16 GB DDR4-3200 (Dual Channel, 2x8GB) | 16 GB LPDDR4x/DDR4 (Dual Channel) | 8 GB VirtIO RAM |
| **GPU** | NVIDIA GeForce RTX 3060 (12GB) / AMD Radeon RX 6600 (8GB) | Integrated Radeon Graphics / Intel Xe Graphics | QEMU VirtIO-GPU / Virgl / PCIe Passthrough |
| **Storage** | 512 GB NVMe PCIe 3.0 SSD (Samsung 970 EVO Plus equivalent) | 512 GB NVMe PCIe SSD | Raw QCOW2 image on NVMe host backing |
| **Firmware** | UEFI 2.8+ 64-bit, Secure Boot Disabled (dev mode), ACPI 6.3+ | UEFI 2.8+ 64-bit, Secure Boot Disabled | OVMF (Open Virtual Machine Firmware) |
| **Display** | 1920x1080 @ 144 Hz (DisplayPort 1.4 / HDMI 2.0) | 1920x1080 @ 60 Hz | 1920x1080 Virtual Framebuffer |

### 4.2 Windows 11 Comparison Configuration
To establish a rigorous baseline, the reference Windows installation must adhere to strict standardization:
- **Operating System**: Windows 11 Pro 64-bit (Version 23H2 or 24H2), clean bare-metal install.
- **Configuration**:
  - Out-of-box default background services running (Windows Defender, Search Indexer, Telemetry, OneDrive sync).
  - Windows Game Mode: Enabled (default user state).
  - Power Scheme: "Balanced" (default) alongside a secondary test with "High Performance".
  - OEM vendor bloatware: Completely absent.
  - GPU Driver: Latest Game Ready / WHQL-certified driver matching hardware.
- **Measurement Protocol**: Windows performance baseline is captured using PresentMon / CapFrameX over identical benchmark timedemo runs.

---

## 5. Metrics, Telemetry & Provisional Budgets

The acceptance criteria establish quantifiable targets across storage, boot, idle memory, and runtime game pacing:

| Metric Category | Target Metric | Windows 11 Baseline (Ref A) | KntonOS Provisional Budget | Validation Methodology |
| :--- | :--- | :--- | :--- | :--- |
| **Storage** | Bootable ISO image size | ~5.8 GB – 6.4 GB | **$\le$ 500 MB** | File size of production `.iso` build |
| **Storage** | Minimal installed root footprint | ~24 GB – 32 GB | **$\le$ 2.0 GB** | On-disk filesystem usage (`kntonfs` root partition) |
| **Boot Time** | UEFI Hand-off to Interactive Desktop | 18.0 s – 25.0 s (cold boot) | **$\le$ 10.0 s** (target: $\le$ 5.0 s) | High-precision hardware timer (TSC / ACPI PM) |
| **Boot Time** | Desktop Idle to Game Interactive | 8.0 s – 12.0 s | **$\le$ 5.0 s** | Process launch to first swapchain image presentation |
| **Memory** | Idle OS RAM (Clean Desktop) | 3,200 MB – 4,400 MB | **$\le$ 300 MB** (target: $\le$ 180 MB) | Kernel PMM active allocated frames (excluding page cache) |
| **Memory** | Total In-Game OS Contention | 1,200 MB – 2,000 MB (non-game) | **$\le$ 150 MB** non-game overhead | Total active physical memory minus game process RSS |
| **Game FPS** | Mean Frame Rate ($FPS_{avg}$) | $X$ FPS (Baseline = 100%) | **$\ge$ 100% of Win11** (Target: 105%–115%) | Hardware swapchain presentation timestamps |
| **Game Stability** | 1% Low Frame Rate ($FPS_{1\%}$) | Typically ~65%–75% of mean | **$\ge$ 90% of mean FPS** | Frame time distribution over 99th percentile |
| **Game Stability** | 0.1% Low Frame Rate ($FPS_{0.1\%}$) | Typically ~40%–55% of mean | **$\ge$ 75% of mean FPS** | Extreme frame drop suppression (eliminates micro-stutter) |
| **Frame Variance** | Frame Time Jitter ($\sigma$) | $\sigma \approx 3.5\text{ ms} - 6.0\text{ ms}$ | **$\sigma \le 1.5\text{ ms}$** | Standard deviation of frame times during 60s benchmark |
| **System Reliability**| Unhandled fault / crash rate | 0 across 10 runs | **0 faults across 10 runs** | Kernel ring buffer, GPU hang detection, core dump check |

---

## 6. Staged Milestone Gating & Dependency Hierarchy

To ensure acceptance gates reflect implementable engineering progress rather than unbacked future claims, acceptance is divided into three concrete stages. The real-game performance gate is strictly decoupled from the initial boot/shell slice:

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│ STAGE 1: Minimal UEFI Kernel & Core User-Space Slice                            │
│ Gating: GATE-01 (Boot to Shell), GATE-02 (Console PE), GATE-03 (Idle RAM <=300M)│
│         GATE-06 (Boot Time <=10s), GATE-07 (ISO Size <=500M)                    │
│ Dependencies: kboot.efi, Ring 0 Rust kernel, PMM/VMM, GOP Framebuffer           │
└─────────────────────────────────────────────────────────────────────────────────┘
                                       │
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────────┐
│ STAGE 2: Interactive Win11-like GUI Desktop & 2D Win32 Execution                │
│ Gating: Windowing Compositor, 60 FPS shell pacing, Win32 GUI PE (.exe)          │
│ Dependencies: kcompositor, kshell, user32/gdi32 emulation, event input          │
└─────────────────────────────────────────────────────────────────────────────────┘
                                       │
                                       ▼  ◄── [HARD DEPENDENCY: GPU Driver Bring-Up]
┌─────────────────────────────────────────────────────────────────────────────────┐
│ STAGE 3: Hardware-Accelerated 3D Public Gaming Demo                             │
│ Gating: GATE-04 (Game FPS >= Win11), GATE-05 (1% Low >= 90%, jitter <=1.5ms)   │
│ Dependencies: Native GPU Driver (Intel/AMD/VirtIO) + Vulkan ICD + Audio Engine  │
└─────────────────────────────────────────────────────────────────────────────────┘
```

### 6.1 Gating Decision Matrix

| Stage | Gate ID | Acceptance Condition | Severity | Hard Prerequisites |
| :--- | :--- | :--- | :--- | :--- |
| **Stage 1** | **GATE-01** | Cold boot to interactive user-space shell without panic or fallback. | Blocker | UEFI 2.8+ loader, PML4 paging, LAPIC timer. |
| **Stage 1** | **GATE-02** | PE32+ console executable executes and outputs to stdout. | Blocker | PE/COFF loader, `VirtualAlloc`, `WriteFile`. |
| **Stage 1** | **GATE-03** | Idle OS RAM footprint $\le 300\text{ MB}$ (excluding page cache). | Blocker | Bitmap/Buddy PMM frame accounting. |
| **Stage 1** | **GATE-06** | Cold boot time $\le 10.0\text{ s}$ from UEFI handoff to shell idle. | Minor | Fast parallel kernel initialization. |
| **Stage 1** | **GATE-07** | Bootable distribution ISO size $\le 500\text{ MB}$. | Minor | Stripped binary packaging, compressed initrd. |
| **Stage 2** | **GATE-08** | Desktop Compositor displays Windows 11-style taskbar and panel. | Major | UEFI GOP framebuffer, 2D blit/rasterizer. |
| **Stage 2** | **GATE-09** | Win32 graphical executable (e.g. 2D calculator/editor) runs. | Major | `user32.dll` message dispatch, window surfaces. |
| **Stage 3** | **GATE-04** | Real-game mean FPS on KntonOS $\ge 100\%$ of identical Windows 11 rig. | Major | **Native GPU Kernel Driver + Vulkan 1.0+ ICD**. |
| **Stage 3** | **GATE-05** | 1% Low FPS $\ge 90\%$ of mean; frame jitter $\sigma \le 1.5\text{ ms}$; zero hangs. | Major | **GPU command ring submission + Direct Scanout**. |

> [!IMPORTANT]
> **Stage 3 Gating Prerequisite**: The real-game performance gate (`GATE-04` and `GATE-05`) CANNOT be evaluated during Stage 1 or Stage 2. It requires an operational GPU command submission driver (e.g., bare-metal AMD/Intel DRM/KMS equivalent or VirtIO-GPU 3D acceleration in QEMU) and a working Vulkan runtime. Stage 1 and Stage 2 deliverables are fully accepted upon satisfying `GATE-01` through `GATE-03` and `GATE-06` through `GATE-09`.

---

## 7. Repeatable Testing Protocol & Variance Control

All benchmark runs must be strictly reproducible by any team member. 

### 7.1 Environmental Control
1. **Thermal Stabilization**:
   - Hardware must undergo a 3-minute pre-warming loop prior to test recording to ensure CPU/GPU clocks settle into steady-state thermal throttle conditions.
   - Ambient room temperature maintained at $21^\circ\text{C} \pm 2^\circ\text{C}$.
2. **Cold vs. Warm Execution Handling**:
   - **Run 0 (Discarded)**: Cold execution for disk cache population, dynamic linking resolution, and shader cache compilation.
   - **Runs 1 through 5 (Recorded)**: Consecutive identical automated timedemo passes.
3. **Network & Background Isolation**:
   - Network interface disabled or disconnected during benchmark runs to eliminate asynchronous network I/O interrupts.
   - Background daemons suspended; only core OS compositor and kernel services active.

### 7.2 Data Acquisition & Statistical Processing
For each candidate run:
1. Capture raw presentation timestamps using the OS hardware display subsystem:
   $$\Delta t_i = t_{presented}(i) - t_{presented}(i-1)$$
2. Compute instantaneous frame rate: $FPS_i = \frac{10^9}{\Delta t_{i\ (\text{ns})}}$.
3. Sort frame times ascending: $\{\Delta t_{(1)}, \Delta t_{(2)}, \dots, \Delta t_{(N)}\}$.
4. Calculate 1% low ($P_{99}$ frame time) and 0.1% low ($P_{99.9}$ frame time):
   $$FPS_{1\%} = \frac{1}{\Delta t_{(\lfloor 0.99 N \rfloor)}}, \quad FPS_{0.1\%} = \frac{1}{\Delta t_{(\lfloor 0.999 N \rfloor)}}$$
5. Calculate sample mean ($\mu$) and standard deviation ($\sigma$):
   $$\mu = \frac{1}{N}\sum_{i=1}^N FPS_i, \quad \sigma = \sqrt{\frac{1}{N-1}\sum_{i=1}^N (FPS_i - \mu)^2}$$
6. Verify Variance Condition: If $\frac{\sigma}{\mu} > 0.03$ (3% coefficient of variation) across Runs 1–5, the test series is flagged as invalid and must be re-run after diagnosing external system interrupts.

### 7.3 Artifact Logging Standards
All benchmark sessions must automatically export structured artifacts to `build/benchmarks/<timestamp>/`:
- `summary.json`: High-level summary containing git commit hash, hardware UUID, ambient metrics, mean FPS, 1% low, 0.1% low, idle RAM, and boot elapsed time.
- `frametimes.csv`: Full time series containing columns: `frame_index`, `timestamp_ns`, `frametime_ms`, `gpu_busy_ms`.
- `memory_trace.log`: Periodic snapshots (100 ms intervals) of kernel physical page allocator counters (`total_frames`, `allocated_frames`, `slab_reclaimable`, `userspace_rss`).
- `kernel_dmesg.txt`: Complete boot and runtime kernel log verifying zero memory leaks, ring buffer overflows, or driver faults.

---

## 8. Interfaces with Downstream Engineering Tracks

This specification directly dictates constraints and validation criteria for subsequent tasks:
1. **TASK-KNT-003 (Platform & Compatibility ADR)**: Must select an architecture capable of hosting the Vulkan user-mode driver and Win32 PE execution without exceeding the 300 MB idle RAM budget.
2. **Kernel Memory Subsystem (Stage 1)**: Must support physical frame accounting, page eviction telemetry, and high-resolution timer (`RDTSC` / ACPI timer) interfaces for timestamp generation.
3. **Compositor & Display Subsystem (Stage 2)**: Must provide a low-overhead presentation queue with direct scanout / zero-copy presentation support to eliminate desktop compositing frame drops.
4. **Toolchain & CI Fixtures (Jules Martin / TASK-KNT-005 & TASK-KNT-006)**: Automated test runner must ingest `summary.json` and enforce Stage 1/2 resource regression guards in automated QEMU test passes.
