# SPEC-001 — First-Demo Resource and Performance Acceptance Plan

| Field | Value |
| --- | --- |
| Document ID | SPEC-001 |
| Task Association | TASK-KNT-004 (First-demo resource and performance acceptance plan) |
| Architecture Tier | Quality Assurance, Benchmarking & Acceptance Criteria |
| Status | Proposed (Provisional Engineering Baseline) |
| Author | Ada Chen (@AdaChen) |
| Reviewer | Marcus (@Marcus, Senior Lead) |
| Target Milestone | First Demonstrable KntonOS Vertical Slice (Native Boot + Real Game) |

---

## 1. Executive Summary & Objective

The strategic charter of KntonOS is to provide an independently developed, lightweight operating system delivering a modern Windows 11-like desktop experience with significantly reduced resource consumption and superior gaming performance for Windows `.exe` applications.

To validate these claims quantitatively rather than qualitatively, this specification establishes:
1. **Representative game/application workloads** for the initial demonstrable milestone.
2. **Reference hardware profiles** and baseline Windows 11 comparison configurations.
3. **Provisional resource and performance budgets** (boot time, idle RAM, ISO size, FPS, and frame-time percentiles).
4. **Repeatable benchmarking protocols**, variance-control procedures, and artifact-logging standards.
5. **Clear governance distinction** separating owner-approved strategic targets from provisional engineering budgets.

---

## 2. Governance: Owner Directives vs. Provisional Engineering Budgets

Per `docs/project-scope.md` and repository guidelines, engineering assumptions must never be disguised as owner requirements.

```
+---------------------------------------------------------------------------------+
|                       OWNER DIRECTIVES (Strategic Mandate)                      |
| - Custom kernel, custom GUI, and core applications                             |
| - Exceptionally lightweight desktop with Windows 11-like feel                   |
| - Broad Windows .exe compatibility target, prioritizing games                   |
| - First public demo: Boot KntonOS and run a real game with improved performance |
| - Licensing: All Rights Reserved (ARR) with selective MIT components            |
+---------------------------------------------------------------------------------+
                                       |
                                       v
+---------------------------------------------------------------------------------+
|                  PROVISIONAL ENGINEERING BUDGETS (This Document)                |
| - ISO footprint: < 500 MB base installation image                               |
| - Cold boot time: < 10.0 seconds from UEFI handoff to desktop idle              |
| - Idle OS memory: < 300 MB resident memory (excluding page cache)              |
| - Frame pacing: 1% low FPS >= 90% of mean; 0.1% low FPS >= 75% of mean          |
| - Game performance: >= 100% (target +5% to +15%) of Windows 11 FPS on same rig  |
+---------------------------------------------------------------------------------+
```

- **Owner-Confirmed Directives**:
  - General-purpose OS with preference for gamers and everyday users.
  - Independently developed native x86-64 UEFI kernel (no modified Linux/BSD/NT kernel).
  - Windows `.exe` game execution on bare metal.
  - Demonstrated, measurable performance improvement over Windows.
- **Provisional Engineering Budgets**:
  - The numerical thresholds specified in Section 4 are engineering proposals designed to establish a concrete, falsifiable pass/fail gate. They remain provisional until validated against physical hardware benchmarks and formally signed off by the project owner.

---

## 3. Workload Selection: Representative Benchmark Candidates

To demonstrate real-world utility while avoiding DRM and licensing entanglements during development, the benchmarking harness defines a three-tiered candidate suite:

### 3.1 Primary Candidate: Open-Source 3D Indie Title (*SuperTuxKart* Windows Release)
- **Application & Version**: *SuperTuxKart* v1.4 (x86-64 Windows Portable Release, `supertuxkart.exe`).
- **Engine / API**: Custom 3D engine using OpenGL / Vulkan (via MoltenVK/Zink or native Vulkan backend) with Direct3D 9/11 wrapper support.
- **Workload & Scene**: 
  - Standardized automated benchmark timedemo: `supertuxkart.exe --bench 120 --track volcano_island --num-karts 8 --reverse`.
  - Fixed 120-second camera loop with dynamic physics, AI agents, particle effects, and particle/mesh rendering.
- **Licensing & Redistribution**: GNU General Public License v3 (GPLv3) for code; Creative Commons for assets. Freely redistributable within internal development environments without proprietary licensing friction.
- **Why Selected**: Tests a complete, non-synthetic game architecture (asset loading, audio thread, physics simulation, multi-threaded rendering, input handling) packaged as a native Windows PE32+ `.exe`.

### 3.2 Secondary Candidate: DRM-Free Commercial Title (*Quake II* / *OpenArena* / *vkQuake*)
- **Application & Version**: *vkQuake* v1.3x (x86-64 PE executable) running Id Software shareware/demo PAK files.
- **Engine / API**: Pure Vulkan rendering backend over standard Win32 windowing and input APIs (`user32.dll`, `kernel32.dll`, `vulkan-1.dll`).
- **Workload & Scene**: Timedemo replay of `demo1.dem` (standardized 60-second combat run at 1080p and 1440p).
- **Licensing**: GPL source executable with royalty-free shareware game data.
- **Why Selected**: Isolates pure Vulkan draw-call overhead and Win32 PE subsystem performance without anti-cheat or proprietary wrapper interference.

### 3.3 Diagnostic / Baseline Candidate: Khronos & Sascha Willems Vulkan PE Test Suite
- **Application & Version**: Sascha Willems Vulkan Examples Suite (`vulkan_gears.exe`, `vulkan_deferred.exe`) compiled as Win64 PE binaries.
- **Workload**: Fixed 10,000-frame automated offscreen/onscreen rendering sequence.
- **Purpose**: Low-level validation fixture for testing kernel memory mapping, GPU command ring submissions, and PE loader link-time relocation before running game physics engines.

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

The acceptance gate requires satisfying measurable budgets across storage, boot, idle memory, and runtime game pacing:

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

## 6. Repeatable Testing Protocol & Variance Control

All benchmark runs must be strictly reproducible by any team member. 

### 6.1 Environmental Control
1. **Thermal Stabilization**:
   - Hardware must undergo a 3-minute pre-warming loop prior to test recording to ensure CPU/GPU clocks settle into steady-state thermal throttle conditions.
   - Ambient room temperature maintained at $21^\circ\text{C} \pm 2^\circ\text{C}$.
2. **Cold vs. Warm Execution Handling**:
   - **Run 0 (Discarded)**: Cold execution for disk cache population, dynamic linking resolution, and shader cache compilation.
   - **Runs 1 through 5 (Recorded)**: Consecutive identical automated timedemo passes.
3. **Network & Background Isolation**:
   - Network interface disabled or disconnected during benchmark runs to eliminate asynchronous network I/O interrupts.
   - Background daemons suspended; only core OS compositor and kernel services active.

### 6.2 Data Acquisition & Statistical Processing
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

### 6.3 Artifact Logging Standards
All benchmark sessions must automatically export structured artifacts to `build/benchmarks/<timestamp>/`:
- `summary.json`: High-level summary containing git commit hash, hardware UUID, ambient metrics, mean FPS, 1% low, 0.1% low, idle RAM, and boot elapsed time.
- `frametimes.csv`: Full time series containing columns: `frame_index`, `timestamp_ns`, `frametime_ms`, `gpu_busy_ms`.
- `memory_trace.log`: Periodic snapshots (100 ms intervals) of kernel physical page allocator counters (`total_frames`, `allocated_frames`, `slab_reclaimable`, `userspace_rss`).
- `kernel_dmesg.txt`: Complete boot and runtime kernel log verifying zero memory leaks, ring buffer overflows, or driver faults.

---

## 7. Pass/Fail Decision Matrix for Demo Readiness

A build is certified **Demo Ready** if and only if all critical gating conditions are satisfied:

| ID | Criteria Description | Gate Level | Condition |
| :--- | :--- | :--- | :--- |
| **GATE-01** | Complete Boot Sequence | **Blocker** | System boots from cold power-on to usable desktop shell without user intervention or fallback shell. |
| **GATE-02** | Unmodified PE Execution | **Blocker** | Selected game `.exe` launches from clean desktop icon/CLI and runs to benchmark completion. |
| **GATE-03** | Idle RAM Ceiling | **Blocker** | Idle desktop RAM consumption $\le 300\text{ MB}$. |
| **GATE-04** | Frame Rate Parity | **Major** | Mean game FPS on KntonOS $\ge 100\%$ of identical Windows 11 reference rig. |
| **GATE-05** | Stutter Suppression | **Major** | 1% low FPS $\ge 90\%$ of mean FPS; zero stutter events $> 50\text{ ms}$ throughout benchmark. |
| **GATE-06** | Boot Time Target | **Minor** | Cold boot time from UEFI exit to desktop interactive state $\le 10.0\text{ seconds}$. |
| **GATE-07** | ISO Size Target | **Minor** | Bootable distribution image file size $\le 500\text{ MB}$. |

---

## 8. Interfaces with Downstream Engineering Tracks

This specification directly dictates constraints and validation criteria for subsequent tasks:
1. **TASK-KNT-003 (Platform & Compatibility ADR)**: Must select an architecture capable of hosting the Vulkan user-mode driver and Win32 PE execution without exceeding the 300 MB idle RAM budget.
2. **Kernel Memory Subsystem**: Must support physical frame accounting, page eviction telemetry, and high-resolution timer (`RDTSC` / ACPI timer) interfaces for timestamp generation.
3. **Compositor & Display Subsystem**: Must provide a low-overhead presentation queue with direct scanout / zero-copy presentation support to eliminate desktop compositing frame drops.
4. **Toolchain & CI (Jules Martin)**: Test automation harness must ingest `summary.json` and enforce regression guards in automated emulator/bare-metal test runs.
