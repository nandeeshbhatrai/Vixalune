//! Deterministic test doubles. Compiled only for unit tests; production code never sees them.

use crate::domain::process::{CancellationToken, OutputStream};
use crate::domain::AppResult;
use crate::ports::{CommandExecutor, CommandOutcome, CommandResult, CommandSpec, ExecOptions, ExecutableLocator, LineSink};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone)]
pub struct Step {
    pub outcome: CommandOutcome,
    pub lines: Vec<String>,
    /// Block until the cancellation token fires, then report `Cancelled`.
    pub block_until_cancel: bool,
    pub delay_ms: u64,
}

impl Step {
    pub fn ok(lines: &[&str]) -> Self {
        Self::exit(0, lines)
    }
    pub fn exit(code: i32, lines: &[&str]) -> Self {
        Self { outcome: CommandOutcome::Exited(code), lines: lines.iter().map(|s| s.to_string()).collect(), block_until_cancel: false, delay_ms: 0 }
    }
    pub fn timed_out() -> Self {
        Self { outcome: CommandOutcome::TimedOut, ..Self::ok(&[]) }
    }
    pub fn cancelled() -> Self {
        Self { outcome: CommandOutcome::Cancelled, ..Self::ok(&[]) }
    }
    /// A command that never finishes on its own (an install in progress).
    pub fn hang(lines: &[&str]) -> Self {
        Self { block_until_cancel: true, ..Self::ok(lines) }
    }
    pub fn delayed(mut self, ms: u64) -> Self {
        self.delay_ms = ms;
        self
    }
}

type Handler = Box<dyn Fn(&CommandSpec) -> Step + Send + Sync>;

pub struct MockExecutor {
    handler: Handler,
    calls: Mutex<Vec<CommandSpec>>,
    options: Mutex<Vec<ExecOptions>>,
}

impl MockExecutor {
    pub fn new(handler: impl Fn(&CommandSpec) -> Step + Send + Sync + 'static) -> Self {
        Self { handler: Box::new(handler), calls: Mutex::new(Vec::new()), options: Mutex::new(Vec::new()) }
    }
    pub fn calls(&self) -> Vec<CommandSpec> {
        self.calls.lock().unwrap().clone()
    }
    /// Options passed to `run_in`, in call order.
    pub fn options(&self) -> Vec<ExecOptions> {
        self.options.lock().unwrap().clone()
    }
}

#[async_trait]
impl CommandExecutor for MockExecutor {
    async fn run_in(&self, spec: CommandSpec, opts: ExecOptions, sink: &dyn LineSink, cancel: &CancellationToken) -> AppResult<CommandResult> {
        self.options.lock().unwrap().push(opts);
        self.run(spec, sink, cancel).await
    }

    async fn run(&self, spec: CommandSpec, sink: &dyn LineSink, cancel: &CancellationToken) -> AppResult<CommandResult> {
        let step = (self.handler)(&spec);
        self.calls.lock().unwrap().push(spec);
        for l in &step.lines {
            sink.line(OutputStream::Stdout, l);
        }
        if step.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(step.delay_ms)).await;
        }
        if step.block_until_cancel {
            while !cancel.is_cancelled() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            return Ok(CommandResult { outcome: CommandOutcome::Cancelled, tail: step.lines });
        }
        if cancel.is_cancelled() && matches!(step.outcome, CommandOutcome::Exited(_)) {
            return Ok(CommandResult { outcome: CommandOutcome::Cancelled, tail: step.lines });
        }
        Ok(CommandResult { outcome: step.outcome, tail: step.lines })
    }
}

#[derive(Default)]
pub struct MockLocator {
    map: HashMap<String, PathBuf>,
}

impl MockLocator {
    pub fn add(&mut self, name: &str, path: &str) {
        self.map.insert(name.to_ascii_lowercase(), PathBuf::from(path));
    }
}

impl ExecutableLocator for MockLocator {
    fn locate(&self, file_name: &str, _extra: &[PathBuf]) -> Option<PathBuf> {
        self.map.get(&file_name.to_ascii_lowercase()).cloned()
    }
    fn locate_system(&self, file_name: &str) -> Option<PathBuf> {
        self.map.get(&file_name.to_ascii_lowercase()).cloned()
    }
}

#[derive(Default)]
pub struct VecSink(Mutex<Vec<String>>);

impl VecSink {
    pub fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl LineSink for VecSink {
    fn line(&self, _: OutputStream, text: &str) {
        self.0.lock().unwrap().push(text.to_string());
    }
}

