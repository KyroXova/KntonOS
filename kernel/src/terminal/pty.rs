//! Pseudoterminal (PTY) Master/Slave Multiplexer & Line Discipline.
//!
//! Implements POSIX / Linux compatible PTY pairs (/dev/ptmx master, /dev/pts/N slave):
//! - Termios configuration (c_iflag, c_oflag, c_cflag, c_lflag, c_cc)
//! - Canonical mode line discipline with interactive line editing (backspace, line-kill ^U, EOF ^D)
//! - Raw mode transparent byte-stream passthrough
//! - PTY master/slave bidirectional data queues and multiplexer

use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::memory::Spinlock;

#[allow(non_camel_case_types)]
pub type tcflag_t = u32;
#[allow(non_camel_case_types)]
pub type cc_t = u8;
#[allow(non_camel_case_types)]
pub type speed_t = u32;

pub const NCCS: usize = 32;

// Control character indices in c_cc
pub const VINTR: usize = 0;   // Interrupt ^C
pub const VQUIT: usize = 1;   // Quit ^\
pub const VERASE: usize = 2;  // Backspace / Erase
pub const VKILL: usize = 3;   // Line kill ^U
pub const VEOF: usize = 4;    // End of file ^D
pub const VTIME: usize = 5;
pub const VMIN: usize = 6;
pub const VSTART: usize = 8;  // XON ^Q
pub const VSTOP: usize = 9;   // XOFF ^S
pub const VSUSP: usize = 10;  // Suspend ^Z
pub const VEOL: usize = 11;

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct winsize {
    pub ws_row: u16,
    pub ws_col: u16,
    pub ws_xpixel: u16,
    pub ws_ypixel: u16,
}

impl winsize {
    pub const fn new(ws_col: u16, ws_row: u16) -> Self {
        Self {
            ws_col,
            ws_row,
            ws_xpixel: ws_col.saturating_mul(8),
            ws_ypixel: ws_row.saturating_mul(16),
        }
    }
}

pub const TCGETS: u64     = 0x5401;
pub const TCSETS: u64     = 0x5402;
pub const TCSETSW: u64    = 0x5403;
pub const TCSETSF: u64    = 0x5404;
pub const TIOCGPGRP: u64  = 0x540F;
pub const TIOCSPGRP: u64  = 0x5410;
pub const TIOCGWINSZ: u64 = 0x5413;
pub const TIOCSWINSZ: u64 = 0x5414;
pub const TIOCGSID: u64   = 0x5429;
pub const TIOCGPTN: u64   = 0x80045430;
pub const TIOCSPTLCK: u64 = 0x40045431;

// Input flags (c_iflag)
pub const IGNBRK: tcflag_t = 0o000001;
pub const BRKINT: tcflag_t = 0o000002;
pub const IGNPAR: tcflag_t = 0o000004;
pub const PARMRK: tcflag_t = 0o000010;
pub const INPCK: tcflag_t  = 0o000020;
pub const ISTRIP: tcflag_t = 0o000040;
pub const INLCR: tcflag_t  = 0o000100;
pub const IGNCR: tcflag_t  = 0o000200;
pub const ICRNL: tcflag_t  = 0o000400; // Translate carriage return to newline on input
pub const IXON: tcflag_t   = 0o002000;
pub const IXOFF: tcflag_t  = 0o010000;

// Output flags (c_oflag)
pub const OPOST: tcflag_t  = 0o000001; // Enable post-processing of output
pub const ONLCR: tcflag_t  = 0o000004; // Map NL to CR-NL on output
pub const OCRNL: tcflag_t  = 0o000010;
pub const ONOCR: tcflag_t  = 0o000020;
pub const ONLRET: tcflag_t = 0o000040;

// Control flags (c_cflag)
pub const CSIZE: tcflag_t  = 0o000060;
pub const CS8: tcflag_t    = 0o000060; // 8-bit character size
pub const CSTOPB: tcflag_t = 0o000100;
pub const CREAD: tcflag_t  = 0o000200; // Enable receiver
pub const PARENB: tcflag_t = 0o000400;

// Local flags (c_lflag)
pub const ISIG: tcflag_t   = 0o000001; // Enable signals
pub const ICANON: tcflag_t = 0o000002; // Canonical mode (line editing)
pub const ECHO: tcflag_t   = 0o000010; // Echo input characters
pub const ECHOE: tcflag_t  = 0o000020; // Visual erase for backspace
pub const ECHOK: tcflag_t  = 0o000040; // Echo newline after kill
pub const ECHONL: tcflag_t = 0o000100; // Echo newline even if ECHO is off
pub const NOFLSH: tcflag_t = 0o000200;
pub const TOSTOP: tcflag_t = 0o000400;
pub const IEXTEN: tcflag_t = 0o100000;

/// Standard POSIX termios structure controlling terminal line discipline.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Termios {
    pub c_iflag: tcflag_t,
    pub c_oflag: tcflag_t,
    pub c_cflag: tcflag_t,
    pub c_lflag: tcflag_t,
    pub c_line: cc_t,
    pub c_cc: [cc_t; NCCS],
    pub c_ispeed: speed_t,
    pub c_ospeed: speed_t,
}

impl Default for Termios {
    fn default() -> Self {
        Self::default_canonical()
    }
}

impl Termios {
    /// Constructs default interactive canonical mode termios configuration.
    pub fn default_canonical() -> Self {
        let mut cc = [0u8; NCCS];
        cc[VINTR] = 0x03; // ^C
        cc[VQUIT] = 0x1C; // ^\
        cc[VERASE] = 0x7F; // DEL / Backspace
        cc[VKILL] = 0x15; // ^U
        cc[VEOF] = 0x04;  // ^D
        cc[VTIME] = 0;
        cc[VMIN] = 1;
        cc[VSTART] = 0x11; // ^Q
        cc[VSTOP] = 0x13;  // ^S
        cc[VSUSP] = 0x1A;  // ^Z

        Self {
            c_iflag: ICRNL | IXON,
            c_oflag: OPOST | ONLCR,
            c_cflag: CS8 | CREAD,
            c_lflag: ISIG | ICANON | ECHO | ECHOE | ECHOK | IEXTEN,
            c_line: 0,
            c_cc: cc,
            c_ispeed: 38400,
            c_ospeed: 38400,
        }
    }

    /// Constructs raw (non-canonical, non-echoing) termios mode.
    pub fn raw() -> Self {
        let mut t = Self::default_canonical();
        t.c_iflag &= !(IGNBRK | BRKINT | PARMRK | ISTRIP | INLCR | IGNCR | ICRNL | IXON);
        t.c_oflag &= !OPOST;
        t.c_lflag &= !(ECHO | ECHONL | ICANON | ISIG | IEXTEN);
        t.c_cflag &= !(CSIZE | PARENB);
        t.c_cflag |= CS8;
        t.c_cc[VMIN] = 1;
        t.c_cc[VTIME] = 0;
        t
    }

    pub const fn is_canonical(&self) -> bool {
        (self.c_lflag & ICANON) != 0
    }

    pub const fn is_echo(&self) -> bool {
        (self.c_lflag & ECHO) != 0
    }

    pub const fn is_echoe(&self) -> bool {
        (self.c_lflag & ECHOE) != 0
    }

    pub const fn is_sig(&self) -> bool {
        (self.c_lflag & ISIG) != 0
    }

    pub const fn is_onlcr(&self) -> bool {
        (self.c_oflag & ONLCR) != 0 && (self.c_oflag & OPOST) != 0
    }

    pub const fn is_icrnl(&self) -> bool {
        (self.c_iflag & ICRNL) != 0
    }
}

/// Errors returned by PTY operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtyError {
    TableFull,
    InvalidHandle,
    NotFound,
    BrokenPipe,
    WouldBlock,
    InvalidArgument,
}

/// Interactive terminal line discipline engine.
#[derive(Debug, Clone)]
pub struct LineDiscipline {
    pub termios: Termios,
    line_buf: Vec<u8>,
    slave_read_queue: VecDeque<u8>,
    master_echo_queue: VecDeque<u8>,
    eof_pending: bool,
}

impl LineDiscipline {
    pub fn new(termios: Termios) -> Self {
        Self {
            termios,
            line_buf: Vec::with_capacity(256),
            slave_read_queue: VecDeque::with_capacity(1024),
            master_echo_queue: VecDeque::with_capacity(1024),
            eof_pending: false,
        }
    }

    /// Processes bytes coming from the master endpoint (keyboard input).
    pub fn write_from_master(&mut self, data: &[u8]) {
        for &byte in data {
            let mut ch = byte;

            // Map CR to NL on input if ICRNL is enabled
            if ch == b'\r' && self.termios.is_icrnl() {
                ch = b'\n';
            }

            if self.termios.is_canonical() {
                // Canonical line discipline
                if ch == self.termios.c_cc[VERASE] || ch == 0x08 {
                    // Backspace / erase
                    if let Some(_removed) = self.line_buf.pop() {
                        if self.termios.is_echo() {
                            if self.termios.is_echoe() {
                                // Visual backspace sequence: BS + Space + BS
                                self.master_echo_queue.extend(b"\x08 \x08");
                            } else {
                                self.master_echo_queue.push_back(0x08);
                            }
                        }
                    }
                } else if ch == self.termios.c_cc[VKILL] {
                    // Line kill (^U)
                    let count = self.line_buf.len();
                    self.line_buf.clear();
                    if self.termios.is_echo() && self.termios.is_echoe() {
                        for _ in 0..count {
                            self.master_echo_queue.extend(b"\x08 \x08");
                        }
                    }
                } else if ch == self.termios.c_cc[VEOF] {
                    // End-of-file (^D)
                    if self.line_buf.is_empty() {
                        self.eof_pending = true;
                    } else {
                        // Flush pending line without newline
                        self.slave_read_queue.extend(self.line_buf.drain(..));
                    }
                } else if ch == b'\n' {
                    // Newline commits line to slave
                    self.line_buf.push(b'\n');
                    self.slave_read_queue.extend(self.line_buf.drain(..));

                    if self.termios.is_echo() {
                        if self.termios.is_onlcr() {
                            self.master_echo_queue.push_back(b'\r');
                        }
                        self.master_echo_queue.push_back(b'\n');
                    }
                } else {
                    // Regular character
                    self.line_buf.push(ch);
                    if self.termios.is_echo() {
                        self.master_echo_queue.push_back(ch);
                    }
                }
            } else {
                // Raw mode: immediate passthrough
                self.slave_read_queue.push_back(ch);
                if self.termios.is_echo() {
                    self.master_echo_queue.push_back(ch);
                }
            }
        }
    }

    /// Processes bytes coming from the slave endpoint (application output) to master.
    pub fn write_from_slave(&mut self, data: &[u8], master_rx: &mut VecDeque<u8>) {
        if self.termios.is_onlcr() {
            for &byte in data {
                if byte == b'\n' {
                    master_rx.push_back(b'\r');
                    master_rx.push_back(b'\n');
                } else {
                    master_rx.push_back(byte);
                }
            }
        } else {
            master_rx.extend(data.iter().copied());
        }
    }

    /// Reads data prepared for slave process.
    pub fn read_for_slave(&mut self, buf: &mut [u8]) -> usize {
        let to_read = core::cmp::min(buf.len(), self.slave_read_queue.len());
        for i in 0..to_read {
            buf[i] = self.slave_read_queue.pop_front().unwrap();
        }
        to_read
    }

    /// Pulls echoed bytes directed back to master terminal display.
    pub fn drain_echo_to_master(&mut self, dest: &mut VecDeque<u8>) {
        dest.extend(self.master_echo_queue.drain(..));
    }

    pub fn is_slave_readable(&self) -> bool {
        !self.slave_read_queue.is_empty() || self.eof_pending
    }

    pub fn take_eof(&mut self) -> bool {
        if self.eof_pending && self.slave_read_queue.is_empty() {
            self.eof_pending = false;
            true
        } else {
            false
        }
    }

    pub fn clear(&mut self) {
        self.line_buf.clear();
        self.slave_read_queue.clear();
        self.master_echo_queue.clear();
        self.eof_pending = false;
    }
}

/// Unified PTY master/slave pair state descriptor.
pub struct PtyPair {
    pub id: usize,
    pub slave_name: String,
    pub ldisc: LineDiscipline,
    pub master_rx_queue: VecDeque<u8>,
    pub master_closed: bool,
    pub slave_closed: bool,
    pub winsize: winsize,
    pub foreground_pgrp: u32,
    pub session_id: u32,
    pub locked: bool,
    pub sigwinch_count: usize,
}

impl PtyPair {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            slave_name: format!("/dev/pts/{}", id),
            ldisc: LineDiscipline::new(Termios::default_canonical()),
            master_rx_queue: VecDeque::with_capacity(1024),
            master_closed: false,
            slave_closed: false,
            winsize: winsize::new(80, 25),
            foreground_pgrp: 0,
            session_id: 0,
            locked: false,
            sigwinch_count: 0,
        }
    }

    /// Master writes keyboard data into PTY line discipline.
    pub fn master_write(&mut self, data: &[u8]) -> Result<usize, PtyError> {
        if self.slave_closed {
            return Err(PtyError::BrokenPipe);
        }
        self.ldisc.write_from_master(data);
        self.ldisc.drain_echo_to_master(&mut self.master_rx_queue);
        Ok(data.len())
    }

    /// Master reads terminal display stream (application output + local echo).
    pub fn master_read(&mut self, buf: &mut [u8]) -> Result<usize, PtyError> {
        if self.master_rx_queue.is_empty() {
            if self.slave_closed {
                return Ok(0); // EOF
            }
            return Err(PtyError::WouldBlock);
        }
        let to_read = core::cmp::min(buf.len(), self.master_rx_queue.len());
        for i in 0..to_read {
            buf[i] = self.master_rx_queue.pop_front().unwrap();
        }
        Ok(to_read)
    }

    /// Slave reads processed input from line discipline.
    pub fn slave_read(&mut self, buf: &mut [u8]) -> Result<usize, PtyError> {
        if !self.ldisc.is_slave_readable() {
            if self.master_closed {
                return Ok(0); // EOF
            }
            return Err(PtyError::WouldBlock);
        }
        if self.ldisc.take_eof() {
            return Ok(0); // Explicit EOF (^D)
        }
        let n = self.ldisc.read_for_slave(buf);
        Ok(n)
    }

    /// Slave writes application output directed to master.
    pub fn slave_write(&mut self, data: &[u8]) -> Result<usize, PtyError> {
        if self.master_closed {
            return Err(PtyError::BrokenPipe);
        }
        self.ldisc.write_from_slave(data, &mut self.master_rx_queue);
        Ok(data.len())
    }

    pub fn set_winsize(&mut self, ws: winsize) -> bool {
        let changed = self.winsize.ws_row != ws.ws_row || self.winsize.ws_col != ws.ws_col;
        self.winsize = ws;
        if changed {
            self.sigwinch_count += 1;
            if self.foreground_pgrp > 0 {
                let _ = crate::process::signals::SIGNAL_MANAGER.lock().send_signal(
                    self.foreground_pgrp as u64,
                    crate::process::signals::SIGWINCH,
                );
            }
        }
        changed
    }

    pub const fn get_winsize(&self) -> winsize {
        self.winsize
    }

    pub fn set_pgrp(&mut self, pgrp: u32) {
        self.foreground_pgrp = pgrp;
    }

    pub const fn get_pgrp(&self) -> u32 {
        self.foreground_pgrp
    }

    pub fn ioctl(&mut self, is_master: bool, cmd: u64, arg: u64) -> Result<i64, PtyError> {
        match cmd {
            TIOCGWINSZ => {
                if arg != 0 {
                    unsafe {
                        let ptr = arg as *mut winsize;
                        *ptr = self.winsize;
                    }
                }
                Ok(0)
            }
            TIOCSWINSZ => {
                if arg != 0 {
                    let new_ws = unsafe { *(arg as *const winsize) };
                    self.set_winsize(new_ws);
                }
                Ok(0)
            }
            TCGETS => {
                if arg != 0 {
                    unsafe {
                        let ptr = arg as *mut Termios;
                        *ptr = self.ldisc.termios;
                    }
                }
                Ok(0)
            }
            TCSETS | TCSETSW | TCSETSF => {
                if arg != 0 {
                    let new_t = unsafe { *(arg as *const Termios) };
                    self.ldisc.termios = new_t;
                }
                Ok(0)
            }
            TIOCGPGRP => {
                if arg != 0 {
                    unsafe {
                        let ptr = arg as *mut u32;
                        *ptr = self.foreground_pgrp;
                    }
                }
                Ok(self.foreground_pgrp as i64)
            }
            TIOCSPGRP => {
                let pgrp = if arg != 0 {
                    unsafe { *(arg as *const u32) }
                } else {
                    0
                };
                self.foreground_pgrp = pgrp;
                Ok(0)
            }
            TIOCGSID => {
                if arg != 0 {
                    unsafe {
                        let ptr = arg as *mut u32;
                        *ptr = self.session_id;
                    }
                }
                Ok(self.session_id as i64)
            }
            TIOCGPTN => {
                if !is_master {
                    return Err(PtyError::InvalidArgument);
                }
                if arg != 0 {
                    unsafe {
                        let ptr = arg as *mut u32;
                        *ptr = self.id as u32;
                    }
                }
                Ok(self.id as i64)
            }
            TIOCSPTLCK => {
                if !is_master {
                    return Err(PtyError::InvalidArgument);
                }
                let lock_flag = if arg != 0 {
                    unsafe { *(arg as *const i32) }
                } else {
                    0
                };
                self.locked = lock_flag != 0;
                Ok(0)
            }
            _ => Err(PtyError::InvalidArgument),
        }
    }
}

/// Global PTY Master/Slave Multiplexer managing /dev/ptmx and /dev/pts/N.
pub struct PtyMultiplexer {
    next_id: usize,
    pairs: BTreeMap<usize, Arc<Spinlock<PtyPair>>>,
}

impl PtyMultiplexer {
    pub const fn new() -> Self {
        Self {
            next_id: 0,
            pairs: BTreeMap::new(),
        }
    }

    /// Allocates a new PTY pair through /dev/ptmx, returning ID, slave name, and pair ref.
    pub fn alloc_pty(&mut self) -> Result<(usize, String, Arc<Spinlock<PtyPair>>), PtyError> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);

        let pair = Arc::new(Spinlock::new(PtyPair::new(id)));
        let name = pair.lock().slave_name.clone();
        self.pairs.insert(id, pair.clone());

        Ok((id, name, pair))
    }

    /// Retrieves an existing PTY pair by ID.
    pub fn get_pty(&self, id: usize) -> Option<Arc<Spinlock<PtyPair>>> {
        self.pairs.get(&id).cloned()
    }

    /// Closes and deallocates a PTY pair.
    pub fn close_pty(&mut self, id: usize) -> Result<(), PtyError> {
        if let Some(pair) = self.pairs.remove(&id) {
            let mut p = pair.lock();
            p.master_closed = true;
            p.slave_closed = true;
            Ok(())
        } else {
            Err(PtyError::NotFound)
        }
    }

    pub fn count(&self) -> usize {
        self.pairs.len()
    }
    pub fn set_winsize(&self, id: usize, ws: winsize) -> Result<bool, PtyError> {
        let pair = self.pairs.get(&id).ok_or(PtyError::NotFound)?;
        Ok(pair.lock().set_winsize(ws))
    }

    pub fn get_winsize(&self, id: usize) -> Result<winsize, PtyError> {
        let pair = self.pairs.get(&id).ok_or(PtyError::NotFound)?;
        Ok(pair.lock().get_winsize())
    }

    pub fn ioctl(&self, id: usize, is_master: bool, cmd: u64, arg: u64) -> Result<i64, PtyError> {
        let pair = self.pairs.get(&id).ok_or(PtyError::NotFound)?;
        pair.lock().ioctl(is_master, cmd, arg)
    }
}

/// Global PTY multiplexer singleton.
pub static PTY_MULTIPLEXER: Spinlock<PtyMultiplexer> = Spinlock::new(PtyMultiplexer::new());


pub fn pty_set_winsize(id: usize, ws: winsize) -> Result<bool, PtyError> {
    PTY_MULTIPLEXER.lock().set_winsize(id, ws)
}

pub fn pty_get_winsize(id: usize) -> Result<winsize, PtyError> {
    PTY_MULTIPLEXER.lock().get_winsize(id)
}

pub fn pty_ioctl(id: usize, is_master: bool, cmd: u64, arg: u64) -> Result<i64, PtyError> {
    PTY_MULTIPLEXER.lock().ioctl(id, is_master, cmd, arg)
}