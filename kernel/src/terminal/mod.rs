//! Virtual Terminal ANSI 24-bit TrueColor Terminal Emulator & PTY Master/Slave Subsystem.
//!
//! Provides full POSIX pseudoterminal pairs (/dev/ptmx, /dev/pts/N), line discipline
//! with canonical and raw modes, ANSI / VT100 / xterm escape code parsing, 24-bit RGB TrueColor SGR,
//! and terminal window sizing with SIGWINCH broadcast.

pub mod ansi;
pub mod pty;

pub use pty::{
    cc_t, speed_t, tcflag_t, LineDiscipline, PtyError, PtyMultiplexer, PtyPair, Termios,
    BRKINT, CREAD, CS8, CSIZE, CSTOPB, ECHO, ECHOE, ECHOK, ECHONL, ICANON, ICRNL, IEXTEN,
    IGNBRK, IGNCR, IGNPAR, INLCR, INPCK, ISIG, ISTRIP, IXOFF, IXON, NCCS, NOFLSH, OCRNL,
    ONLCR, ONLRET, ONOCR, OPOST, PARENB, PARMRK, PTY_MULTIPLEXER, TOSTOP, VEOF, VEOL, VERASE,
    VINTR, VKILL, VMIN, VQUIT, VSTART, VSTOP, VSUSP, VTIME, winsize,
    TCGETS, TCSETS, TCSETSW, TCSETSF, TIOCGPGRP, TIOCSPGRP, TIOCGWINSZ, TIOCSWINSZ,
    TIOCGSID, TIOCGPTN, TIOCSPTLCK, pty_ioctl, pty_set_winsize, pty_get_winsize,
};

pub use ansi::{
    xterm_256_to_color, AnsiCell, AnsiCursor, AnsiStyle, AnsiTerminal, ANSI_16_COLORS,
};

use crate::display::color::Color;

/// Verification test suite for PTY master/slave multiplexer and ANSI 24-bit TrueColor terminal emulator.
pub fn test_pty_and_ansi_subsystem() -> bool {
    crate::serial_write_str("[KNT-TERM-PTY] Running PTY master/slave and ANSI 24-bit TrueColor verification suite...\r\n");

    // 1. PTY Allocation, Master/Slave Bidirectional Communication & Echo
    let (pty_id, slave_name, pty_pair) = match PTY_MULTIPLEXER.lock().alloc_pty() {
        Ok(res) => res,
        Err(_) => {
            crate::serial_write_str("[KNT-PANIC] Failed to allocate PTY pair from multiplexer!\r\n");
            return false;
        }
    };

    if !slave_name.starts_with("/dev/pts/") {
        crate::serial_write_str("[KNT-PANIC] Slave naming format mismatch!\r\n");
        return false;
    }

    {
        let mut p = pty_pair.lock();

        // 1.1 Master writes string ending in \n -> verifies line discipline echo to master
        let input_line = b"echo Hello KntonOS PTY\n";
        if p.master_write(input_line).is_err() {
            crate::serial_write_str("[KNT-PANIC] master_write failed!\r\n");
            return false;
        }

        // Master reads back echo: should contain the echoed input line with \r\n (due to ONLCR)
        let mut echo_buf = [0u8; 64];
        let echo_len = p.master_read(&mut echo_buf).unwrap_or(0);
        if echo_len == 0 || !echo_buf[..echo_len].starts_with(b"echo Hello KntonOS PTY") {
            crate::serial_write_str("[KNT-PANIC] PTY master echo verification mismatch!\r\n");
            return false;
        }

        // 1.2 Slave reads the line from line discipline
        let mut slave_buf = [0u8; 64];
        let slave_len = p.slave_read(&mut slave_buf).unwrap_or(0);
        if slave_len != input_line.len() || &slave_buf[..slave_len] != input_line {
            crate::serial_write_str("[KNT-PANIC] PTY slave read line content mismatch!\r\n");
            return false;
        }

        // 1.3 Slave writes application output -> master reads it back
        let app_output = b"Process completed.\n";
        if p.slave_write(app_output).is_err() {
            crate::serial_write_str("[KNT-PANIC] slave_write failed!\r\n");
            return false;
        }

        let mut out_buf = [0u8; 64];
        let out_len = p.master_read(&mut out_buf).unwrap_or(0);
        // With ONLCR, \n becomes \r\n
        if out_len == 0 || !out_buf[..out_len].starts_with(b"Process completed.\r\n") {
            crate::serial_write_str("[KNT-PANIC] Slave-to-master output or ONLCR mismatch!\r\n");
            return false;
        }

        crate::serial_write_str("  * PTY allocation, bidirectional data flow, and local echo verified\r\n");
    }

    // 2. Line Discipline Canonical Editing: Backspace, Line-Kill (^U), and EOF (^D)
    {
        let mut p = pty_pair.lock();

        // 2.1 Backspace (0x7F or 0x08)
        let bs_seq = b"abcde\x7F\x7FXY\n";
        if p.master_write(bs_seq).is_err() {
            crate::serial_write_str("[KNT-PANIC] Backspace write failed!\r\n");
            return false;
        }
        let mut slv_buf = [0u8; 32];
        let slv_len = p.slave_read(&mut slv_buf).unwrap_or(0);
        if slv_len != 6 || &slv_buf[..slv_len] != b"abcXY\n" {
            crate::serial_write_str("[KNT-PANIC] Line discipline backspace editing mismatch!\r\n");
            return false;
        }
        let mut drain_buf = [0u8; 64];
        while p.master_read(&mut drain_buf).is_ok() {}

        // 2.2 Line Kill (0x15 / ^U)
        let kill_seq = b"mistaken command\x15correct\n";
        if p.master_write(kill_seq).is_err() {
            crate::serial_write_str("[KNT-PANIC] Line-kill write failed!\r\n");
            return false;
        }
        let mut slv_buf2 = [0u8; 32];
        let slv_len2 = p.slave_read(&mut slv_buf2).unwrap_or(0);
        if slv_len2 != 8 || &slv_buf2[..slv_len2] != b"correct\n" {
            crate::serial_write_str("[KNT-PANIC] Line discipline line-kill (^U) editing mismatch!\r\n");
            return false;
        }
        while p.master_read(&mut drain_buf).is_ok() {}

        // 2.3 EOF (^D = 0x04)
        let eof_seq = b"\x04";
        if p.master_write(eof_seq).is_err() {
            crate::serial_write_str("[KNT-PANIC] EOF write failed!\r\n");
            return false;
        }
        let mut eof_buf = [0u8; 16];
        let eof_len = p.slave_read(&mut eof_buf).unwrap_or(usize::MAX);
        if eof_len != 0 {
            crate::serial_write_str("[KNT-PANIC] Expected EOF (0 bytes read) from line discipline ^D!\r\n");
            return false;
        }

        crate::serial_write_str("  * Line discipline canonical editing (erase, kill, EOF) verified\r\n");
    }

    // 3. ANSI / VT100 / xterm 24-bit TrueColor SGR & Cursor Control State Machine
    {
        let mut term = AnsiTerminal::new(80, 25);

        // 3.1 Plain text output
        term.process_bytes(b"KntonOS");
        if term.cursor.col != 7 || term.cursor.row != 0 {
            crate::serial_write_str("[KNT-PANIC] Terminal cursor advance mismatch!\r\n");
            return false;
        }

        // 3.2 24-bit TrueColor Foreground (CSI 38;2;255;128;64 m) & Background (CSI 48;2;16;32;48 m)
        term.process_bytes(b"\x1B[38;2;255;128;64;48;2;16;32;48mColorTest\x1B[0m");

        if let Some(cell) = term.cell(7, 0) {
            if cell.ch != 'C' {
                crate::serial_write_str("[KNT-PANIC] Character cell mismatch!\r\n");
                return false;
            }
            if cell.style.fg != Color::rgb(255, 128, 64) {
                crate::serial_write_str("[KNT-PANIC] 24-bit TrueColor Foreground mismatch!\r\n");
                return false;
            }
            if cell.style.bg != Color::rgb(16, 32, 48) {
                crate::serial_write_str("[KNT-PANIC] 24-bit TrueColor Background mismatch!\r\n");
                return false;
            }
        } else {
            crate::serial_write_str("[KNT-PANIC] Cell lookup returned None!\r\n");
            return false;
        }

        // 3.3 Extended 256-color palette (CSI 38;5;196 m = Bright Red)
        term.process_bytes(b"\x1B[38;5;196mRedText\x1B[0m");
        let expected_red = xterm_256_to_color(196);
        if let Some(cell) = term.cell(16, 0) {
            if cell.ch != 'R' || cell.style.fg != expected_red {
                crate::serial_write_str("[KNT-PANIC] 256-color palette decoding mismatch!\r\n");
                return false;
            }
        }

        // 3.4 SGR Attributes: Bold, Underline, and Reverse
        term.process_bytes(b"\x1B[1;4;7mAttrText\x1B[0m");
        if let Some(cell) = term.cell(23, 0) {
            if !cell.style.bold || !cell.style.underline || !cell.style.reverse {
                crate::serial_write_str("[KNT-PANIC] SGR attribute flags mismatch!\r\n");
                return false;
            }
        }

        // 3.5 Cursor Navigation (CSI row;col H) and Cursor Save/Restore (CSI s / CSI u)
        term.process_bytes(b"\x1B[10;20H"); // Move to row 10, col 20 (1-indexed -> 9, 19)
        if term.cursor.row != 9 || term.cursor.col != 19 {
            crate::serial_write_str("[KNT-PANIC] Absolute cursor position (CSI H) mismatch!\r\n");
            return false;
        }
        term.process_bytes(b"\x1B[s"); // Save cursor
        term.process_bytes(b"\x1B[5A"); // Move up 5 -> row 4
        if term.cursor.row != 4 {
            crate::serial_write_str("[KNT-PANIC] Cursor Up (CSI A) mismatch!\r\n");
            return false;
        }
        term.process_bytes(b"\x1B[u"); // Restore cursor -> row 9, col 19
        if term.cursor.row != 9 || term.cursor.col != 19 {
            crate::serial_write_str("[KNT-PANIC] Cursor Restore (CSI u) mismatch!\r\n");
            return false;
        }

        crate::serial_write_str("  * ANSI 24-bit TrueColor SGR, 256-color palette, attributes, and cursor navigation verified\r\n");
    }

    // 4. PTY Window Sizing (TIOCGWINSZ / TIOCSWINSZ) & SIGWINCH Signal Propagation
    {
        let initial_ws = match pty_get_winsize(pty_id) {
            Ok(ws) => ws,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to get initial winsize!\r\n");
                return false;
            }
        };
        if initial_ws.ws_col != 80 || initial_ws.ws_row != 25 {
            crate::serial_write_str("[KNT-PANIC] Initial winsize dimensions mismatch!\r\n");
            return false;
        }

        // Set foreground process group to target PID 1 (e.g. init process)
        {
            let mut p = pty_pair.lock();
            p.set_pgrp(1);
        }

        // Resize window to 120 cols x 40 rows
        let new_ws = winsize::new(120, 40);
        let changed = match pty_set_winsize(pty_id, new_ws) {
            Ok(c) => c,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to set new winsize!\r\n");
                return false;
            }
        };

        if !changed {
            crate::serial_write_str("[KNT-PANIC] set_winsize did not report change!\r\n");
            return false;
        }

        let updated_ws = pty_get_winsize(pty_id).unwrap();
        if updated_ws.ws_col != 120 || updated_ws.ws_row != 40 {
            crate::serial_write_str("[KNT-PANIC] Updated winsize mismatch!\r\n");
            return false;
        }

        // Verify SIGWINCH notification count
        let sigwinch_count = pty_pair.lock().sigwinch_count;
        if sigwinch_count != 1 {
            crate::serial_write_str("[KNT-PANIC] SIGWINCH propagation counter mismatch!\r\n");
            return false;
        }

        // Clean up PTY pair
        let _ = PTY_MULTIPLEXER.lock().close_pty(pty_id);
        crate::serial_write_str("  * PTY ioctl window sizing (TIOCGWINSZ/TIOCSWINSZ) & SIGWINCH propagation verified\r\n");
    }

    crate::serial_write_str("[KNT-TERM-PTY-PASS] PTY master/slave multiplexer and ANSI 24-bit TrueColor parser verified\r\n");
    true
}