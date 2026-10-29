//! Built-in shell commands: help, ver, ls, cat, ps, clear.

use alloc::format;
use alloc::vec::Vec;
use core::str;
use crate::fs::{FileType, VFS};
use crate::process::SCHEDULER;
use super::terminal::TerminalBuffer;

pub fn execute_command(line: &str, term: &mut TerminalBuffer) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }

    let mut parts = trimmed.split_whitespace();
    let cmd = parts.next().unwrap_or("");
    let args: Vec<&str> = parts.collect();

    match cmd {
        "help" => cmd_help(term),
        "ver" | "version" => cmd_ver(term),
        "ls" | "dir" => cmd_ls(&args, term),
        "cat" | "type" => cmd_cat(&args, term),
        "ps" | "tasklist" => cmd_ps(term),
        "clear" | "cls" => cmd_clear(term),
        unknown => {
            term.write_str("\x1B[31mUnknown command: \x1B[0m");
            term.write_str(unknown);
            term.write_str("\r\nType 'help' for available commands.\r\n");
        }
    }
}

fn cmd_help(term: &mut TerminalBuffer) {
    term.write_str("\x1B[36;1mKntonOS Virtual Terminal Shell Commands:\x1B[0m\r\n");
    term.write_str("  \x1B[33mhelp\x1B[0m       - Display this list of available shell commands\r\n");
    term.write_str("  \x1B[33mver\x1B[0m        - Display KntonOS version and architecture details\r\n");
    term.write_str("  \x1B[33mls [path]\x1B[0m  - List directory contents from Virtual File System\r\n");
    term.write_str("  \x1B[33mcat <file>\x1B[0m - Output text contents of a file from VFS\r\n");
    term.write_str("  \x1B[33mps\x1B[0m         - List active processes, threads, and scheduler queues\r\n");
    term.write_str("  \x1B[33mclear\x1B[0m      - Clear the terminal screen\r\n");
}

fn cmd_ver(term: &mut TerminalBuffer) {
    term.write_str("\x1B[34;1mKntonOS Micro-Kernel Shell\x1B[0m\r\n");
    term.write_str("Version: 0.1.0-alpha (x86_64-unknown-uefi)\r\n");
    term.write_str("Subsystems: PMM, VMM, VFS, Scheduler, Syscall NT, VirtIO, Acrylic Compositor\r\n");
    term.write_str("License: ARR | Author: Knton OS Contributors\r\n");
}

fn cmd_ls(args: &[&str], term: &mut TerminalBuffer) {
    let path = if args.is_empty() { "/" } else { args[0] };
    let vfs = VFS.lock();

    let node = match vfs.open(path) {
        Ok(n) => n,
        Err(_) => {
            term.write_str("\x1B[31mls: cannot access '");
            term.write_str(path);
            term.write_str("': No such file or directory\x1B[0m\r\n");
            return;
        }
    };

    let meta = node.metadata();
    if meta.file_type != FileType::Directory {
        term.write_str("\x1B[32m[FILE] ");
        let sz = format!("{} B", meta.size);
        term.write_str(&sz);
        term.write_str("  ");
        term.write_str(path);
        term.write_str("\x1B[0m\r\n");
        return;
    }

    let entries = match node.readdir() {
        Ok(e) => e,
        Err(_) => {
            term.write_str("\x1B[31mls: error reading directory entries\x1B[0m\r\n");
            return;
        }
    };

    term.write_str("\x1B[36mTYPE     NAME\x1B[0m\r\n");
    for entry in entries {
        match entry.file_type {
            FileType::Directory => {
                term.write_str("\x1B[34m<DIR>    ");
                term.write_str(&entry.name);
                term.write_str("/\x1B[0m\r\n");
            }
            FileType::File => {
                term.write_str("\x1B[32m<FILE>   ");
                term.write_str(&entry.name);
                term.write_str("\x1B[0m\r\n");
            }
            _ => {
                term.write_str("<DEV>    ");
                term.write_str(&entry.name);
                term.write_str("\r\n");
            }
        }
    }
}

fn cmd_cat(args: &[&str], term: &mut TerminalBuffer) {
    if args.is_empty() {
        term.write_str("Usage: cat <file_path>\r\n");
        return;
    }

    let path = args[0];
    let vfs = VFS.lock();
    match vfs.read_file(path) {
        Ok(bytes) => {
            if let Ok(text) = str::from_utf8(&bytes) {
                term.write_str(text);
                if !text.ends_with('\n') {
                    term.write_str("\r\n");
                }
            } else {
                term.write_str("\x1B[33m[binary file, length ");
                let sz = format!("{}", bytes.len());
                term.write_str(&sz);
                term.write_str(" bytes]\x1B[0m\r\n");
            }
        }
        Err(_) => {
            term.write_str("\x1B[31mcat: ");
            term.write_str(path);
            term.write_str(": No such file or unreadable\x1B[0m\r\n");
        }
    }
}

fn cmd_ps(term: &mut TerminalBuffer) {
    let guard = SCHEDULER.lock();
    let sched = match guard.as_ref() {
        Some(s) => s,
        None => {
            term.write_str("Scheduler not initialized\r\n");
            return;
        }
    };

    term.write_str("\x1B[36mPID   TID   PRIORITY   STATE        NAME\x1B[0m\r\n");

    // Current running thread
    if let Some(ref cur) = sched.current {
        let line = format!(
            "{:<5} {:<5} {:<10?} \x1B[32m{:<12?}\x1B[0m {}\r\n",
            cur.pid, cur.tid, cur.priority, cur.state, cur.name
        );
        term.write_str(&line);
    }

    // Queued ready threads across priorities
    for queue in sched.runqueues.iter().rev() {
        for thread in queue.iter() {
            let line = format!(
                "{:<5} {:<5} {:<10?} {:<12?} {}\r\n",
                thread.pid, thread.tid, thread.priority, thread.state, thread.name
            );
            term.write_str(&line);
        }
    }

    let footer = format!("\x1B[90mTotal context switches: {}\x1B[0m\r\n", sched.switch_count);
    term.write_str(&footer);
}

fn cmd_clear(term: &mut TerminalBuffer) {
    term.clear();
}
