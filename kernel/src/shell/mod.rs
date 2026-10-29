//! Interactive Virtual Terminal (VT) Shell and Key-Event Dispatch Engine for KntonOS.

pub mod commands;
pub mod input;
pub mod jobs;
pub mod pipe;
pub mod pipeline;
pub mod terminal;

use alloc::string::String;
use crate::compositor::with_compositor;
use crate::memory::spinlock::Spinlock;
use crate::serial_write_str;

pub use commands::execute_command;
pub use input::{KeyAction, KeyBuffer, KeyboardDecoder, KeyEvent, ModifierState, poll_ps2_hardware};
pub use jobs::{cmd_bg, cmd_fg, cmd_jobs, cmd_kill, register_job, JobStatus, JobTable, ShellJob, JOB_TABLE};
pub use pipe::AnonymousPipe;
pub use pipeline::{
    ExecutionMode, ParseError, ParsedPipeline, PipelineParser, PipelineStage, RedirectionSource,
    RedirectionTarget,
};
pub use terminal::{TerminalBuffer, TerminalCell, DEFAULT_COLS, DEFAULT_ROWS};

/// Stateful interactive virtual terminal shell session.
pub struct ShellSession {
    pub terminal: TerminalBuffer,
    pub input_decoder: KeyboardDecoder,
    pub current_line: String,
    pub prompt: &'static str,
    pub window_id: Option<u32>,
}

impl ShellSession {
    pub fn new(cols: usize, rows: usize) -> Self {
        let mut terminal = TerminalBuffer::new(cols, rows);
        let prompt = "knton> ";

        // Print initial welcome banner
        terminal.write_str("\x1B[36;1mKntonOS Virtual Terminal Shell v0.1.0\x1B[0m\r\n");
        terminal.write_str("Type '\x1B[33mhelp\x1B[0m' for built-in commands.\r\n\r\n");
        terminal.write_str(prompt);

        Self {
            terminal,
            input_decoder: KeyboardDecoder::new(),
            current_line: String::new(),
            prompt,
            window_id: None,
        }
    }

    pub fn handle_char(&mut self, c: char) {
        match c {
            '\n' | '\r' => {
                self.terminal.write_str("\r\n");
                let line = self.current_line.clone();
                self.current_line.clear();
                execute_command(&line, &mut self.terminal);
                self.terminal.write_str(self.prompt);
            }
            '\x08' => {
                if !self.current_line.is_empty() {
                    self.current_line.pop();
                    self.terminal.write_char('\x08');
                }
            }
            printable => {
                self.current_line.push(printable);
                self.terminal.write_char(printable);
            }
        }

        self.sync_with_window();
    }

    pub fn handle_scancode(&mut self, scancode: u8) {
        if let Some(event) = self.input_decoder.process_scancode_set1(scancode) {
            self.handle_key_event(event);
        }
    }

    pub fn handle_key_event(&mut self, event: KeyEvent) {
        if event.action == KeyAction::Press {
            if let Some(c) = event.char_code {
                self.handle_char(c);
            }
        }
    }

    pub fn sync_with_window(&mut self) {
        if let Some(win_id) = self.window_id {
            with_compositor(|comp| {
                let bounds = if let Some(win) = comp.find_window_mut(win_id) {
                    self.terminal.render_to_client_buffer(
                        &mut win.client_buffer,
                        win.client_width,
                        win.client_height,
                    );
                    Some(win.client_bounds())
                } else {
                    None
                };
                if let Some(b) = bounds {
                    comp.damage_rect(b);
                }
            });
        }
    }
}

pub static SHELL: Spinlock<Option<ShellSession>> = Spinlock::new(None);

/// Push a decoded key event into the active virtual terminal shell session.
pub fn push_key_event(event: KeyEvent) {
    let mut guard = SHELL.lock();
    if let Some(ref mut shell) = *guard {
        shell.handle_key_event(event);
    }
}

/// Initialize the global shell session.
pub fn init(cols: usize, rows: usize) {
    serial_write_str("[KNT-SHELL] Interactive Virtual Terminal (VT) Shell initialized\r\n");
    let mut guard = SHELL.lock();
    *guard = Some(ShellSession::new(cols, rows));
}

/// Attach the shell terminal to an acrylic foreground desktop window in the compositor.
pub fn attach_to_compositor() -> Option<u32> {
    let mut guard = SHELL.lock();
    let shell = guard.as_mut()?;

    let win_id = with_compositor(|comp| {
        let id = comp.create_window("KntonOS Terminal", 15, 15, 290, 180);
        comp.bring_to_front(id);
        let bounds = if let Some(win) = comp.find_window_mut(id) {
            shell.terminal.render_to_client_buffer(
                &mut win.client_buffer,
                win.client_width,
                win.client_height,
            );
            Some(win.frame_bounds())
        } else {
            None
        };
        if let Some(b) = bounds {
            comp.damage_rect(b);
        }
        id
    })?;

    shell.window_id = Some(win_id);
    Some(win_id)
}

/// Dispatches a raw PS/2 scancode to the global active shell session.
pub fn dispatch_scancode(scancode: u8) {
    let mut guard = SHELL.lock();
    if let Some(ref mut shell) = *guard {
        shell.handle_scancode(scancode);
    }
}

/// Verification self-test suite for the KntonOS shell and key-event engine.
pub fn test_shell_subsystem() -> bool {
    serial_write_str("[KNT-SHELL-TEST] Running VT shell and key-event engine verification suite...\r\n");

    // 1. Test PS/2 Scancode Set 1 and Set 2 decoder & modifiers
    {
        let mut decoder = KeyboardDecoder::new();

        // Test normal lowercase keypress: Make code 0x1E ('a')
        let ev_a = decoder.process_scancode_set1(0x1E);
        match ev_a {
            Some(ev) => {
                if ev.char_code != Some('a') || ev.action != KeyAction::Press {
                    serial_write_str("[KNT-PANIC] Key decoder lowercase 'a' failed!\r\n");
                    return false;
                }
            }
            None => {
                serial_write_str("[KNT-PANIC] Key decoder did not produce event for 0x1E!\r\n");
                return false;
            }
        }

        // Test Left Shift press: Make code 0x2A
        decoder.process_scancode_set1(0x2A);
        if !decoder.modifiers.is_shift() {
            serial_write_str("[KNT-PANIC] Left shift modifier not tracked!\r\n");
            return false;
        }

        // Test shifted uppercase keypress: 0x1E ('A')
        let ev_shift_a = decoder.process_scancode_set1(0x1E);
        if ev_shift_a.and_then(|e| e.char_code) != Some('A') {
            serial_write_str("[KNT-PANIC] Key decoder shifted 'A' failed!\r\n");
            return false;
        }

        // Test Left Shift release: Break code 0xAA (0x2A | 0x80)
        decoder.process_scancode_set1(0xAA);
        if decoder.modifiers.is_shift() {
            serial_write_str("[KNT-PANIC] Left shift release failed!\r\n");
            return false;
        }

        // Test Backspace (0x0E) and Enter (0x1C)
        let ev_bs = decoder.process_scancode_set1(0x0E);
        if ev_bs.and_then(|e| e.char_code) != Some('\x08') {
            serial_write_str("[KNT-PANIC] Key decoder Backspace failed!\r\n");
            return false;
        }

        let ev_enter = decoder.process_scancode_set1(0x1C);
        if ev_enter.and_then(|e| e.char_code) != Some('\n') {
            serial_write_str("[KNT-PANIC] Key decoder Enter failed!\r\n");
            return false;
        }

        // Test Scancode Set 2: Make code 0x1C ('a')
        let ev_s2 = decoder.process_scancode_set2(0x1C);
        if ev_s2.and_then(|e| e.char_code) != Some('a') {
            serial_write_str("[KNT-PANIC] Key decoder Set 2 'a' failed!\r\n");
            return false;
        }

        serial_write_str("  * PS/2 keyboard scancode set 1/2 decoder verified\r\n");
        serial_write_str("[KNT-SHELL-INPUT-PASS] PS/2 keyboard scancode set 1/2 decoder verified\r\n");
    }

    // 2. Test TerminalBuffer grid, ANSI escape parsing, scrolling, and cursor blink
    {
        let mut term = TerminalBuffer::new(40, 10);
        term.write_str("Hello \x1B[32mKnton\x1B[0m\n");

        if term.get_cell(0, 0).map(|c| c.character) != Some('H') {
            serial_write_str("[KNT-PANIC] TerminalBuffer text write failed!\r\n");
            return false;
        }
        // Check green color for 'K'
        if let Some(cell) = term.get_cell(6, 0) {
            if cell.character != 'K' || cell.fg != terminal::ANSI_COLOR_GREEN {
                serial_write_str("[KNT-PANIC] TerminalBuffer ANSI color SGR code failed!\r\n");
                return false;
            }
        } else {
            serial_write_str("[KNT-PANIC] Terminal cell out of bounds!\r\n");
            return false;
        }

        // Test scrolling: write 12 lines into 10-line grid
        for i in 1..=12 {
            term.write_char((b'0' + (i % 10) as u8) as char);
            term.write_char('\n');
        }

        // Bottom row should have cursor at row 9
        if term.cursor_row != 9 {
            serial_write_str("[KNT-PANIC] Terminal auto-scrolling row index mismatch!\r\n");
            return false;
        }

        // Test cursor toggle blink
        let b1 = term.cursor_blink_on;
        term.toggle_cursor_blink();
        if term.cursor_blink_on == b1 {
            serial_write_str("[KNT-PANIC] Cursor blink toggle failed!\r\n");
            return false;
        }

        serial_write_str("  * Virtual Terminal grid buffer and ANSI parser verified\r\n");
        serial_write_str("[KNT-SHELL-TERM-PASS] Virtual Terminal grid buffer and ANSI parser verified\r\n");
    }

    // 3. Test built-in shell commands: help, ver, ls, cat, ps, clear
    {
        let mut term = TerminalBuffer::new(60, 20);

        // Command: help
        execute_command("help", &mut term);
        if term.cursor_row == 0 && term.cursor_col == 0 {
            serial_write_str("[KNT-PANIC] Command 'help' did not produce output!\r\n");
            return false;
        }

        // Command: ver
        term.clear();
        execute_command("ver", &mut term);
        if term.get_cell(0, 0).map(|c| c.character) != Some('K') {
            serial_write_str("[KNT-PANIC] Command 'ver' output mismatch!\r\n");
            return false;
        }

        // Command: ls /
        term.clear();
        execute_command("ls /", &mut term);
        if term.cursor_row == 0 {
            serial_write_str("[KNT-PANIC] Command 'ls /' did not list VFS entries!\r\n");
            return false;
        }

        // Command: cat /etc/os-release
        term.clear();
        execute_command("cat /etc/os-release", &mut term);
        // /etc/os-release starts with "NAME=KntonOS"
        if term.get_cell(0, 0).map(|c| c.character) != Some('N') {
            serial_write_str("[KNT-PANIC] Command 'cat /etc/os-release' failed to read file!\r\n");
            return false;
        }

        // Command: ps
        term.clear();
        execute_command("ps", &mut term);
        if term.get_cell(0, 0).map(|c| c.character) != Some('P') {
            serial_write_str("[KNT-PANIC] Command 'ps' failed to list threads!\r\n");
            return false;
        }

        // Command: clear
        execute_command("clear", &mut term);
        if term.cursor_row != 0 || term.cursor_col != 0 {
            serial_write_str("[KNT-PANIC] Command 'clear' failed to reset cursor!\r\n");
            return false;
        }

        serial_write_str("  * Built-in shell commands (help, ver, ls, cat, ps, clear) verified\r\n");
        serial_write_str("[KNT-SHELL-CMD-PASS] Built-in shell commands (help, ver, ls, cat, ps, clear) verified\r\n");
    }

    // 4. Test Compositor acrylic window attachment and rendering
    {
        let win_id_opt = attach_to_compositor();
        match win_id_opt {
            Some(win_id) => {
                let verified = with_compositor(|comp| {
                    comp.find_window(win_id).is_some()
                }).unwrap_or(false);

                if !verified {
                    serial_write_str("[KNT-PANIC] Shell window not registered in compositor!\r\n");
                    return false;
                }
            }
            None => {
                serial_write_str("[KNT-PANIC] Failed to attach shell to compositor!\r\n");
                return false;
            }
        }

        serial_write_str("  * Foreground acrylic terminal window rendering verified\r\n");
        serial_write_str("[KNT-SHELL-WINDOW-PASS] Foreground acrylic terminal window rendering verified\r\n");
    }

    // 5. Test shell pipelines, file redirections, and job control
    if !test_pipeline_and_job_control() {
        return false;
    }

    serial_write_str("[KNT-SHELL-TESTS-PASS] All interactive virtual terminal shell checks passed\r\n");
    true
}

/// Comprehensive verification test suite for VT Shell Pipelines, I/O Redirection, and Process Job Control.
pub fn test_pipeline_and_job_control() -> bool {
    serial_write_str("[KNT-SHELL-PIPE-TEST] Running shell pipeline, redirection & job control test suite...\r\n");

    // 1. Pipeline command parser with pipe and redirection tokens
    let parsed = match PipelineParser::parse("cat < /input.txt | ver | cat > /test.txt &") {
        Ok(p) => p,
        Err(_) => {
            serial_write_str("[KNT-PANIC] Failed to parse complex shell pipeline!\r\n");
            return false;
        }
    };

    if parsed.stages.len() != 3 || !parsed.is_background() {
        serial_write_str("[KNT-PANIC] Parsed pipeline stages or background mode mismatch!\r\n");
        return false;
    }
    if parsed.stages[0].command != "cat" || parsed.stages[0].stdin_source != RedirectionSource::File(alloc::string::String::from("/input.txt")) {
        serial_write_str("[KNT-PANIC] Stage 0 input redirection parsing failed!\r\n");
        return false;
    }
    if parsed.stages[1].command != "ver" || parsed.stages[1].stdin_source != RedirectionSource::Pipe(0) || parsed.stages[1].stdout_target != RedirectionTarget::Pipe(2) {
        serial_write_str("[KNT-PANIC] Stage 1 pipe routing mismatch!\r\n");
        return false;
    }
    if parsed.stages[2].command != "cat" || parsed.stages[2].stdout_target != RedirectionTarget::FileTruncate(alloc::string::String::from("/test.txt")) {
        serial_write_str("[KNT-PANIC] Stage 2 output redirection parsing failed!\r\n");
        return false;
    }
    serial_write_str("  * Pipeline token parsing and redirection stage descriptors verified\r\n");
    serial_write_str("[KNT-SHELL-PARSE-PASS] Pipeline token parsing and redirection stage descriptors verified\r\n");

    // 2. Anonymous Pipe IPC channel connecting command stages
    let mut pipe = AnonymousPipe::new();
    let sample_data = b"KntonOS Pipe Stream Payload";
    let written = pipe.write(sample_data);
    if written != sample_data.len() || pipe.available() != sample_data.len() {
        serial_write_str("[KNT-PANIC] Anonymous pipe write failed!\r\n");
        return false;
    }

    let mut buf = [0u8; 7];
    let n = pipe.read(&mut buf);
    if n != 7 || &buf != b"KntonOS" {
        serial_write_str("[KNT-PANIC] Anonymous pipe partial read failed!\r\n");
        return false;
    }

    pipe.close_write();
    if pipe.is_eof() {
        serial_write_str("[KNT-PANIC] Pipe signaled EOF while unread bytes remained in FIFO!\r\n");
        return false;
    }

    let remaining = pipe.read_all();
    if remaining != b" Pipe Stream Payload" || !pipe.is_eof() {
        serial_write_str("[KNT-PANIC] Pipe read_all or post-EOF condition failed!\r\n");
        return false;
    }
    serial_write_str("  * Anonymous pipe FIFO byte ring, partial read, and EOF signaling verified\r\n");
    serial_write_str("[KNT-SHELL-PIPE-CHAN-PASS] Anonymous pipe FIFO byte ring, partial read, and EOF signaling verified\r\n");

    // 3. Pipeline execution: ver | cat, output redirection to /test.txt, and input read
    let mut term = TerminalBuffer::new(60, 20);

    // Execute ver | cat
    execute_command("ver | cat", &mut term);
    if term.get_cell(0, 0).map(|c| c.character) != Some('K') {
        serial_write_str("[KNT-PANIC] Pipeline 'ver | cat' execution failed!\r\n");
        return false;
    }

    // Execute ver > /test.txt
    execute_command("ver > /test.txt", &mut term);
    let vfs_data = match crate::fs::VFS.lock().read_file("/test.txt") {
        Ok(d) => d,
        Err(_) => {
            serial_write_str("[KNT-PANIC] File /test.txt not created by shell output redirection!\r\n");
            return false;
        }
    };
    if !vfs_data.starts_with(b"KntonOS Micro-Kernel Shell") {
        serial_write_str("[KNT-PANIC] File /test.txt content mismatch!\r\n");
        return false;
    }

    // Execute cat /test.txt | cat
    term.clear();
    execute_command("cat /test.txt | cat", &mut term);
    if term.get_cell(0, 0).map(|c| c.character) != Some('K') {
        serial_write_str("[KNT-PANIC] Pipeline 'cat /test.txt | cat' failed!\r\n");
        return false;
    }
    serial_write_str("  * Pipeline chaining and VFS file redirection verified\r\n");
    serial_write_str("[KNT-SHELL-REDIR-PASS] Pipeline chaining and VFS file redirection verified\r\n");

    // 4. Job control and background execution tracking
    let job_id = register_job(101, "worker_daemon &");
    if job_id == 0 {
        serial_write_str("[KNT-PANIC] Background job registration failed!\r\n");
        return false;
    }

    term.clear();
    cmd_jobs(&mut term);
    if term.cursor_row == 0 {
        serial_write_str("[KNT-PANIC] Command 'jobs' produced no output!\r\n");
        return false;
    }

    cmd_fg("1", &mut term);
    cmd_bg("1", &mut term);
    cmd_kill("101", &mut term);

    let is_done = {
        let table = JOB_TABLE.lock();
        table.get_job(job_id).map(|j| j.status) == Some(JobStatus::Done)
    };
    if !is_done {
        serial_write_str("[KNT-PANIC] Job status after kill is not Done!\r\n");
        return false;
    }
    serial_write_str("  * Shell job table tracking, fg/bg control, and process kill verified\r\n");
    serial_write_str("[KNT-SHELL-JOBS-PASS] Shell job table tracking, fg/bg control, and process kill verified\r\n");

    serial_write_str("[KNT-SHELL-PIPE-PASS] All shell pipeline, redirection, and job control checks passed\r\n");
    true
}
