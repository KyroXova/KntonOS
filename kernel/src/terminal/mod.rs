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