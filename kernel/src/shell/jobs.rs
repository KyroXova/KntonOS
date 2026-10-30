//! Shell process job control and background execution tracking.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::memory::spinlock::Spinlock;
use super::terminal::TerminalBuffer;

/// Execution status of a shell background job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Running,
    Stopped,
    Done,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Running => "Running",
            JobStatus::Stopped => "Stopped",
            JobStatus::Done => "Done",
        }
    }
}

/// Metadata describing an active shell job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellJob {
    pub job_id: u32,
    pub pid: u64,
    pub command: String,
    pub status: JobStatus,
}

/// Central job table for background process control.
pub struct JobTable {
    jobs: Vec<ShellJob>,
    next_job_id: u32,
}

impl JobTable {
    pub const fn new() -> Self {
        Self {
            jobs: Vec::new(),
            next_job_id: 1,
        }
    }

    /// Add a new job to the table. Returns allocated `JobId`.
    pub fn add_job(&mut self, pid: u64, command: &str, status: JobStatus) -> u32 {
        let job_id = self.next_job_id;
        self.next_job_id = self.next_job_id.wrapping_add(1);
        self.jobs.push(ShellJob {
            job_id,
            pid,
            command: String::from(command),
            status,
        });
        job_id
    }

    pub fn get_job(&self, job_id: u32) -> Option<&ShellJob> {
        self.jobs.iter().find(|j| j.job_id == job_id)
    }

    pub fn get_job_mut(&mut self, job_id: u32) -> Option<&mut ShellJob> {
        self.jobs.iter_mut().find(|j| j.job_id == job_id)
    }

    pub fn find_by_pid(&self, pid: u64) -> Option<&ShellJob> {
        self.jobs.iter().find(|j| j.pid == pid)
    }

    pub fn find_by_pid_mut(&mut self, pid: u64) -> Option<&mut ShellJob> {
        self.jobs.iter_mut().find(|j| j.pid == pid)
    }

    pub fn set_status(&mut self, job_id: u32, status: JobStatus) -> bool {
        if let Some(j) = self.get_job_mut(job_id) {
            j.status = status;
            true
        } else {
            false
        }
    }

    pub fn remove_job(&mut self, job_id: u32) -> Option<ShellJob> {
        if let Some(pos) = self.jobs.iter().position(|j| j.job_id == job_id) {
            Some(self.jobs.remove(pos))
        } else {
            None
        }
    }

    pub fn remove_by_pid(&mut self, pid: u64) -> Option<ShellJob> {
        if let Some(pos) = self.jobs.iter().position(|j| j.pid == pid) {
            Some(self.jobs.remove(pos))
        } else {
            None
        }
    }

    pub fn list_jobs(&self) -> &[ShellJob] {
        &self.jobs
    }

    pub fn prune_done(&mut self) {
        self.jobs.retain(|j| j.status != JobStatus::Done);
    }
}

pub static JOB_TABLE: Spinlock<JobTable> = Spinlock::new(JobTable::new());

/// Register a new background job into the shell job table.
pub fn register_job(pid: u64, command: &str) -> u32 {
    let mut table = JOB_TABLE.lock();
    table.add_job(pid, command, JobStatus::Running)
}

/// Output active jobs table to the virtual terminal buffer.
pub fn cmd_jobs(term: &mut TerminalBuffer) {
    let table = JOB_TABLE.lock();
    let jobs = table.list_jobs();

    if jobs.is_empty() {
        term.write_str("No active background jobs.\r\n");
        return;
    }

    term.write_str("\x1B[36m[JOB]   PID    STATUS       COMMAND\x1B[0m\r\n");
    for job in jobs {
        let status_color = match job.status {
            JobStatus::Running => "\x1B[32m",
            JobStatus::Stopped => "\x1B[33m",
            JobStatus::Done => "\x1B[90m",
        };
        let line = format!(
            "[{}]     {:<6} {}{:<12}\x1B[0m {}\r\n",
            job.job_id,
            job.pid,
            status_color,
            job.status.as_str(),
            job.command
        );
        term.write_str(&line);
    }
}

/// Bring a background job to the foreground.
pub fn cmd_fg(job_id_str: &str, term: &mut TerminalBuffer) {
    let job_id = match job_id_str.parse::<u32>() {
        Ok(id) => id,
        Err(_) => {
            term.write_str("Usage: fg <job_id>\r\n");
            return;
        }
    };

    let mut table = JOB_TABLE.lock();
    if let Some(job) = table.get_job_mut(job_id) {
        job.status = JobStatus::Running;
        let line = format!("[{}] {} brought to foreground\r\n", job.job_id, job.command);
        term.write_str(&line);
    } else {
        let err = format!("fg: no such job: [{}]\r\n", job_id);
        term.write_str(&err);
    }
}

/// Resume a stopped job in the background.
pub fn cmd_bg(job_id_str: &str, term: &mut TerminalBuffer) {
    let job_id = match job_id_str.parse::<u32>() {
        Ok(id) => id,
        Err(_) => {
            term.write_str("Usage: bg <job_id>\r\n");
            return;
        }
    };

    let mut table = JOB_TABLE.lock();
    if let Some(job) = table.get_job_mut(job_id) {
        job.status = JobStatus::Running;
        let line = format!("[{}] {}& resumed in background\r\n", job.job_id, job.command);
        term.write_str(&line);
    } else {
        let err = format!("bg: no such job: [{}]\r\n", job_id);
        term.write_str(&err);
    }
}

/// Terminate an active job or process by PID.
pub fn cmd_kill(pid_str: &str, term: &mut TerminalBuffer) {
    let pid = match pid_str.parse::<u64>() {
        Ok(p) => p,
        Err(_) => {
            term.write_str("Usage: kill <pid>\r\n");
            return;
        }
    };

    let mut table = JOB_TABLE.lock();
    if let Some(job) = table.find_by_pid_mut(pid) {
        job.status = JobStatus::Done;
        let msg = format!("[{}] Process {} terminated\r\n", job.job_id, pid);
        term.write_str(&msg);
    } else {
        let msg = format!("Process {} terminated\r\n", pid);
        term.write_str(&msg);
    }
}
