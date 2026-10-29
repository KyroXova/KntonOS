# TASK-KNT-009 — Virtual File System (VFS) and Memory-Mapped Initramfs

| Field | Value |
| --- | --- |
| Status | Ready |
| Requirement/source | `docs/architecture/ADR-001-native-platform-and-compatibility.md` Slice 1 & Slice 4 |
| Milestone | A hierarchical Virtual File System (VFS) and in-memory initramfs (Tar/CPIO) reader capable of mounting root `/` and reading files into memory. |
| Depends on | TASK-KNT-006 |
| Owner/session | Priya Nair / dev |
| Components | `kernel/src/fs/` |
| Review | Marcus verifies trait abstraction, path resolution, and zero-panic boundary. |

## Goal and scope

Implement the foundational VFS layer for KntonOS:
1. **VFS Core**:
   - `INode` and `VfsNode` abstractions (`File`, `Directory`, `Symlink`, `Device`).
   - File operations: `open()`, `read()`, `write()`, `close()`, `lookup()`, `readdir()`.
   - File modes & permissions (`FilePerms`, `SeekMode`).
2. **Initramfs / Ramdisk Driver**:
   - In-memory archive parser (UStar / CPIO format) parsing a boot-time memory slice into a mountable directory tree at `/`.
   - Path resolution: `resolve_path("/system/bin/init") -> Result<VfsNode, FsError>`.
3. **Integration**:
   - Compiles cleanly in `#![no_std]` bare-metal mode under `x86_64-unknown-none`.

## Acceptance

- [ ] Path resolution handles absolute and relative paths (`/`, `/a/b`, `..`, `.`).
- [ ] In-memory file read returns exact bytes.
- [ ] Compiles with zero errors under `cargo check`.
- [ ] Atomic commits posted to AgentIRC `#dev` and evidence in `#reports`.
