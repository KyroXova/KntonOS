//! Pipeline command parser, redirection tokens, and execution stage descriptors.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    EmptyCommandLine,
    EmptyPipelineStage,
    MissingRedirectionTarget,
    UnexpectedToken,
}

/// Source of standard input for a pipeline stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectionSource {
    StandardInput,
    File(String),
    Pipe(usize), // Stage index supplying input stream
}

/// Destination of standard output for a pipeline stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectionTarget {
    StandardOutput,
    FileTruncate(String),
    FileAppend(String),
    Pipe(usize), // Next stage index receiving output stream
}

/// Process execution mode in shell session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Foreground,
    Background,
}

/// Describes a single command stage in a pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineStage {
    pub stage_index: usize,
    pub command: String,
    pub args: Vec<String>,
    pub stdin_source: RedirectionSource,
    pub stdout_target: RedirectionTarget,
    pub execution_mode: ExecutionMode,
}

/// Parsed multi-stage command pipeline with I/O routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPipeline {
    pub stages: Vec<PipelineStage>,
    pub execution_mode: ExecutionMode,
    pub raw_line: String,
}

impl ParsedPipeline {
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    #[inline(always)]
    pub fn is_piped(&self) -> bool {
        self.stages.len() > 1
    }

    #[inline(always)]
    pub fn is_background(&self) -> bool {
        self.execution_mode == ExecutionMode::Background
    }
}

/// Parser for shell command pipelines and I/O redirection tokens.
pub struct PipelineParser;

impl PipelineParser {
    pub fn parse(line: &str) -> Result<ParsedPipeline, ParseError> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Err(ParseError::EmptyCommandLine);
        }

        // 1. Check for background execution flag '&' at the end
        let (command_part, execution_mode) = if trimmed.ends_with('&') {
            let without_amp = trimmed[..trimmed.len() - 1].trim();
            if without_amp.is_empty() {
                return Err(ParseError::EmptyCommandLine);
            }
            (without_amp, ExecutionMode::Background)
        } else {
            (trimmed, ExecutionMode::Foreground)
        };

        // 2. Split into pipeline segments separated by '|'
        let pipe_segments: Vec<&str> = command_part.split('|').collect();
        let total_stages = pipe_segments.len();

        let mut stages = Vec::with_capacity(total_stages);

        for (idx, &segment) in pipe_segments.iter().enumerate() {
            let seg_trimmed = segment.trim();
            if seg_trimmed.is_empty() {
                return Err(ParseError::EmptyPipelineStage);
            }

            let stage = Self::parse_stage(seg_trimmed, idx, total_stages, execution_mode)?;
            stages.push(stage);
        }

        Ok(ParsedPipeline {
            stages,
            execution_mode,
            raw_line: String::from(trimmed),
        })
    }

    /// Parse a single pipeline command segment with input/output redirections.
    fn parse_stage(
        segment: &str,
        stage_index: usize,
        total_stages: usize,
        execution_mode: ExecutionMode,
    ) -> Result<PipelineStage, ParseError> {
        let tokens = Self::tokenize(segment);
        if tokens.is_empty() {
            return Err(ParseError::EmptyPipelineStage);
        }

        let mut command = String::new();
        let mut args = Vec::new();

        let mut stdin_source = if stage_index > 0 {
            RedirectionSource::Pipe(stage_index - 1)
        } else {
            RedirectionSource::StandardInput
        };

        let mut stdout_target = if stage_index + 1 < total_stages {
            RedirectionTarget::Pipe(stage_index + 1)
        } else {
            RedirectionTarget::StandardOutput
        };

        let mut i = 0;
        while i < tokens.len() {
            let token = &tokens[i];
            if token == "<" {
                if i + 1 >= tokens.len() {
                    return Err(ParseError::MissingRedirectionTarget);
                }
                stdin_source = RedirectionSource::File(tokens[i + 1].clone());
                i += 2;
            } else if token == ">" {
                if i + 1 >= tokens.len() {
                    return Err(ParseError::MissingRedirectionTarget);
                }
                stdout_target = RedirectionTarget::FileTruncate(tokens[i + 1].clone());
                i += 2;
            } else if token == ">>" {
                if i + 1 >= tokens.len() {
                    return Err(ParseError::MissingRedirectionTarget);
                }
                stdout_target = RedirectionTarget::FileAppend(tokens[i + 1].clone());
                i += 2;
            } else {
                if command.is_empty() {
                    command = token.clone();
                } else {
                    args.push(token.clone());
                }
                i += 1;
            }
        }

        if command.is_empty() {
            return Err(ParseError::EmptyPipelineStage);
        }

        Ok(PipelineStage {
            stage_index,
            command,
            args,
            stdin_source,
            stdout_target,
            execution_mode,
        })
    }

    /// Split segment into tokens respecting quotes (`"` and `'`) and redirection operators.
    fn tokenize(segment: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let chars: Vec<char> = segment.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            // Skip leading whitespace
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }
            if i >= len {
                break;
            }

            // Check for redirection tokens
            if chars[i] == '>' {
                if i + 1 < len && chars[i + 1] == '>' {
                    tokens.push(String::from(">>"));
                    i += 2;
                } else {
                    tokens.push(String::from(">"));
                    i += 1;
                }
                continue;
            } else if chars[i] == '<' {
                tokens.push(String::from("<"));
                i += 1;
                continue;
            }

            // Quoted string
            if chars[i] == '"' || chars[i] == '\'' {
                let quote = chars[i];
                i += 1;
                let mut current = String::new();
                while i < len && chars[i] != quote {
                    current.push(chars[i]);
                    i += 1;
                }
                if i < len && chars[i] == quote {
                    i += 1; // Consume closing quote
                }
                tokens.push(current);
                continue;
            }

            // Word token
            let mut current = String::new();
            while i < len && !chars[i].is_whitespace() && chars[i] != '>' && chars[i] != '<' {
                current.push(chars[i]);
                i += 1;
            }
            if !current.is_empty() {
                tokens.push(current);
            }
        }

        tokens
    }
}

use super::pipe::AnonymousPipe;
use super::terminal::TerminalBuffer;
use crate::fs::VFS;

impl ParsedPipeline {
    /// Execute pipeline stages chained via anonymous pipes and file redirections.
    pub fn execute(&self, term: &mut TerminalBuffer) {
        if self.stages.is_empty() {
            return;
        }

        let stage_count = self.stages.len();
        let mut pipes = Vec::with_capacity(stage_count);
        for _ in 0..stage_count {
            pipes.push(AnonymousPipe::new());
        }

        for (i, stage) in self.stages.iter().enumerate() {
            // 1. Resolve input data for this stage
            let input_bytes = match &stage.stdin_source {
                RedirectionSource::Pipe(prev_idx) => {
                    if *prev_idx < pipes.len() {
                        pipes[*prev_idx].read_all()
                    } else {
                        Vec::new()
                    }
                }
                RedirectionSource::File(path) => {
                    VFS.lock().read_file(path).unwrap_or_default()
                }
                RedirectionSource::StandardInput => Vec::new(),
            };

            // 2. Execute command stage, capturing output
            let stage_output = Self::run_stage_command(stage, &input_bytes, term);

            // 3. Direct output to destination
            match &stage.stdout_target {
                RedirectionTarget::Pipe(_next_idx) => {
                    if i < pipes.len() {
                        pipes[i].write(&stage_output);
                        pipes[i].close_write();
                    }
                }
                RedirectionTarget::FileTruncate(path) => {
                    VFS.lock().write_file(path, &stage_output);
                }
                RedirectionTarget::FileAppend(path) => {
                    VFS.lock().append_file(path, &stage_output);
                }
                RedirectionTarget::StandardOutput => {
                    if let Ok(text) = core::str::from_utf8(&stage_output) {
                        term.write_str(text);
                    }
                }
            }
        }
    }

    /// Internal executor for an individual stage command with standard input/output.
    pub fn run_stage_command(
        stage: &PipelineStage,
        input: &[u8],
        _term: &mut TerminalBuffer,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let args_ref: Vec<&str> = stage.args.iter().map(|s| s.as_str()).collect();

        match stage.command.as_str() {
            "ver" | "version" => {
                out.extend_from_slice(b"KntonOS Micro-Kernel Shell\r\nVersion: 0.1.0-alpha (x86_64-unknown-uefi)\r\nSubsystems: PMM, VMM, VFS, Scheduler, Syscall NT, VirtIO, Acrylic Compositor\r\n");
            }
            "cat" | "type" => {
                if args_ref.is_empty() || args_ref[0] == "-" {
                    out.extend_from_slice(input);
                } else {
                    let vfs = VFS.lock();
                    for path in args_ref {
                        if let Ok(data) = vfs.read_file(path) {
                            out.extend_from_slice(&data);
                        } else {
                            let err_msg = alloc::format!("cat: {}: No such file\r\n", path);
                            out.extend_from_slice(err_msg.as_bytes());
                        }
                    }
                }
            }
            "echo" => {
                let mut first = true;
                for a in &stage.args {
                    if !first {
                        out.push(b' ');
                    }
                    first = false;
                    out.extend_from_slice(a.as_bytes());
                }
                out.extend_from_slice(b"\r\n");
            }
            "ls" | "dir" => {
                let path = if args_ref.is_empty() { "/" } else { args_ref[0] };
                let vfs = VFS.lock();
                if let Ok(node) = vfs.open(path) {
                    let meta = node.metadata();
                    if meta.file_type == crate::fs::FileType::Directory {
                        if let Ok(entries) = node.readdir() {
                            out.extend_from_slice(b"TYPE     NAME\r\n");
                            for entry in entries {
                                let line = alloc::format!("{:<8} {}\r\n", if entry.file_type == crate::fs::FileType::Directory { "<DIR>" } else { "<FILE>" }, entry.name);
                                out.extend_from_slice(line.as_bytes());
                            }
                        }
                    } else {
                        let line = alloc::format!("[FILE] {} B {}\r\n", meta.size, path);
                        out.extend_from_slice(line.as_bytes());
                    }
                } else {
                    let err = alloc::format!("ls: cannot access '{}': No such file or directory\r\n", path);
                    out.extend_from_slice(err.as_bytes());
                }
            }
            "help" => {
                out.extend_from_slice(b"KntonOS Virtual Terminal Shell Commands:\r\n  help, ver, ls, cat, echo, ps, jobs, fg, bg, kill, clear\r\n");
            }
            unknown => {
                let err = alloc::format!("Unknown command: {}\r\n", unknown);
                out.extend_from_slice(err.as_bytes());
            }
        }

        out
    }
}

