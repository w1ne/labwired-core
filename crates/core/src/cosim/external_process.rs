use crate::cosim::{CosimAdapter, CosimStep, CosimStepResult};
#[cfg(unix)]
use crate::cosim::shm::{self, ShmRegion};
#[cfg(unix)]
use crate::cosim::{CosimSignalValue, CosimSignals};
use crate::{SimResult, SimulationError};
#[cfg(unix)]
use serde_json::{Map, Value};
#[cfg(unix)]
use std::io::Write;
use std::time::Duration;
#[cfg(unix)]
use std::{
    io::Read,
    os::fd::AsRawFd,
    os::unix::process::CommandExt,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    time::Instant,
};

/// Native Unix JSONL limits. One deadline covers encoding, writing and reading;
/// no worker threads, queues, retries or process replacement are used.
#[derive(Clone, Copy, Debug)]
pub struct ExternalProcessCosimOptions {
    pub timeout: Duration,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
}
impl Default for ExternalProcessCosimOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(2),
            max_request_bytes: 65_536,
            max_response_bytes: 65_536,
        }
    }
}

pub struct ExternalProcessCosimAdapter {
    #[cfg(unix)]
    child: Option<Child>,
    #[cfg(unix)]
    stdin: Option<ChildStdin>,
    #[cfg(unix)]
    stdout: Option<ChildStdout>,
    options: ExternalProcessCosimOptions,
    failed: bool,
    /// Present once the child accepted the shared-memory transport; steps then
    /// bypass the pipes entirely.
    #[cfg(unix)]
    shm: Option<ShmLink>,
}

#[cfg(unix)]
struct ShmLink {
    region: ShmRegion,
    in_names: Vec<String>,
    out_names: Vec<String>,
}
fn error(message: impl Into<String>) -> SimulationError {
    SimulationError::Other(message.into())
}
impl ExternalProcessCosimAdapter {
    pub fn spawn(program: &str, args: &[&str]) -> SimResult<Self> {
        Self::spawn_with_options(program, args, ExternalProcessCosimOptions::default())
    }
    pub fn spawn_with_options(
        program: &str,
        args: &[&str],
        options: ExternalProcessCosimOptions,
    ) -> SimResult<Self> {
        Self::spawn_inner(program, args, options, None)
    }

    /// Spawn the child offering the shared-memory transport for the named model
    /// inputs and outputs. If the child announces support within `handshake`
    /// the adapter uses it; a child that stays silent is a plain JSONL model
    /// and keeps working unchanged. Check [`Self::uses_shm`].
    pub fn spawn_shm(
        program: &str,
        args: &[&str],
        options: ExternalProcessCosimOptions,
        in_names: &[String],
        out_names: &[String],
        handshake: Duration,
    ) -> SimResult<Self> {
        #[cfg(not(unix))]
        {
            let _ = (in_names, out_names, handshake);
            Self::spawn_inner(program, args, options, None)
        }
        #[cfg(unix)]
        {
            let region = ShmRegion::create(in_names, out_names)
                .map_err(|err| error(format!("co-sim shm create failed: {err}")))?;
            let mut adapter = Self::spawn_inner(program, args, options, Some(&region))?;
            match adapter.await_hello(handshake) {
                Ok(true) => {
                    // Both sides hold the mapping; the name is no longer needed.
                    region.unlink();
                    adapter.shm = Some(ShmLink {
                        in_names: region.in_names().to_vec(),
                        out_names: region.out_names().to_vec(),
                        region,
                    });
                    Ok(adapter)
                }
                Ok(false) => Ok(adapter), // region dropped and unlinked: JSONL.
                Err(err) => {
                    adapter.cleanup();
                    Err(err)
                }
            }
        }
    }

    /// True once the shared-memory transport is active for this child.
    pub fn uses_shm(&self) -> bool {
        #[cfg(unix)]
        {
            self.shm.is_some()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    #[cfg(unix)]
    fn await_hello(&mut self, handshake: Duration) -> SimResult<bool> {
        let deadline = Instant::now() + handshake;
        let stdout = self
            .stdout
            .as_mut()
            .ok_or_else(|| error("missing co-sim stdout"))?;
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return if line.is_empty() {
                    Ok(false)
                } else {
                    Err(error("co-sim shm handshake was cut off"))
                };
            }
            let millis = remaining.as_millis().max(1).min(i32::MAX as u128) as i32;
            let mut descriptor = libc::pollfd {
                fd: stdout.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut descriptor, 1, millis) };
            if ready <= 0 {
                continue;
            }
            match stdout.read(&mut byte) {
                Ok(0) => return Err(error("co-sim process exited during shm handshake")),
                Ok(_) => {
                    if byte[0] == b'\n' {
                        return if line == shm::HELLO_LINE.as_bytes() {
                            Ok(true)
                        } else {
                            Err(error("unexpected output from co-sim process before first step"))
                        };
                    }
                    line.push(byte[0]);
                    if line.len() > 256 {
                        return Err(error("co-sim shm handshake line too long"));
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(error(format!("co-sim read failed: {e}"))),
            }
        }
    }

    fn spawn_inner(
        program: &str,
        args: &[&str],
        options: ExternalProcessCosimOptions,
        #[cfg(unix)] region: Option<&ShmRegion>,
        #[cfg(not(unix))] _region: Option<()>,
    ) -> SimResult<Self> {
        // Hard ceilings keep even caller-selected settings bounded.
        if options.timeout.is_zero()
            || options.timeout > Duration::from_secs(60)
            || !(128..=1_048_576).contains(&options.max_request_bytes)
            || !(128..=1_048_576).contains(&options.max_response_bytes)
        {
            return Err(error(
                "invalid co-sim IPC bounds (timeout 0..60s, frames 128..1048576 bytes)",
            ));
        }
        #[cfg(not(unix))]
        {
            let _ = (program, args, options);
            Err(SimulationError::NotImplemented("bounded external-process co-simulation requires native Unix; unsupported on browser/Windows".into()))
        }
        #[cfg(unix)]
        {
            let mut command = Command::new(program);
            command.args(args);
            if let Some(region) = region {
                command.env(shm::ENV_PATH, region.path());
            }
            let mut child = command
                .process_group(0)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .map_err(|err| {
                    error(format!("failed to spawn co-sim process '{program}': {err}"))
                })?;
            let stdin = child.stdin.take();
            let stdout = child.stdout.take();
            let mut adapter = Self {
                child: Some(child),
                stdin,
                stdout,
                options,
                failed: false,
                shm: None,
            };
            let result = (|| {
                let stdin = adapter
                    .stdin
                    .as_ref()
                    .ok_or_else(|| error("missing co-sim stdin"))?;
                let stdout = adapter
                    .stdout
                    .as_ref()
                    .ok_or_else(|| error("missing co-sim stdout"))?;
                nonblocking(stdin.as_raw_fd())?;
                nonblocking(stdout.as_raw_fd())?;
                Ok(())
            })();
            if let Err(err) = result {
                adapter.cleanup();
                return Err(err);
            }
            Ok(adapter)
        }
    }
    fn cleanup(&mut self) {
        self.failed = true;
        #[cfg(unix)]
        if let Some(mut link) = self.shm.take() {
            link.region.close();
        }
        #[cfg(unix)]
        if let Some(mut child) = self.child.take() {
            // process_group(0) creates our own group before exec. Kill it BEFORE
            // reaping the leader, which also handles descendants inheriting pipes.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            self.stdin.take();
            self.stdout.take();
            // Never wait() indefinitely (e.g. a child in kernel uninterruptible
            // sleep). Normal killed children are reaped here; cleanup is capped.
            let until = Instant::now() + Duration::from_millis(250);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) => {}
                }
                if Instant::now() >= until {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
    #[cfg(unix)]
    fn shm_exchange(
        &mut self,
        time_ns: u64,
        dt_ns: u64,
        inputs: &CosimSignals,
        outputs: &mut CosimSignals,
    ) -> SimResult<()> {
        let deadline = Instant::now() + self.options.timeout;
        let Self { child, shm, .. } = self;
        let link = shm.as_mut().ok_or_else(|| error("co-sim process failed"))?;
        let ShmLink {
            region,
            in_names,
            out_names,
        } = link;
        let mut matched = 0usize;
        for (index, name) in in_names.iter().enumerate() {
            let slot = match inputs.get(name.as_str()) {
                None => None,
                Some(value) => {
                    matched += 1;
                    Some(match value {
                        CosimSignalValue::Bool(b) => (shm::KIND_BOOL, *b as u64),
                        CosimSignalValue::I64(i) => (shm::KIND_I64, *i as u64),
                        CosimSignalValue::F64(f) if f.is_finite() => (shm::KIND_F64, f.to_bits()),
                        CosimSignalValue::F64(_) => return Err(error("nonfinite co-sim input")),
                        CosimSignalValue::Text(_) => {
                            return Err(error(
                                "the shm co-sim transport carries bool, integer and float \
                                 signals only; a text input needs transport: jsonl",
                            ))
                        }
                    })
                }
            };
            region.set_input(index, slot);
        }
        if matched != inputs.len() {
            return Err(error("co-sim input not declared for the shm transport"));
        }
        region
            .exchange(time_ns, dt_ns, deadline, || {
                child
                    .as_mut()
                    .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
            })
            .map_err(error)?;
        if region.fault() != 0 {
            return Err(error("co-sim process reported a model fault"));
        }
        for (index, name) in out_names.iter().enumerate() {
            match region.output(index) {
                None => {
                    outputs.remove(name.as_str());
                }
                Some((kind, bits)) => {
                    let value = match kind {
                        shm::KIND_BOOL => CosimSignalValue::Bool(bits != 0),
                        shm::KIND_I64 => CosimSignalValue::I64(bits as i64),
                        shm::KIND_F64 => {
                            let f = f64::from_bits(bits);
                            if !f.is_finite() {
                                return Err(error("nonfinite co-sim output"));
                            }
                            CosimSignalValue::F64(f)
                        }
                        _ => return Err(error("unknown co-sim shm signal kind")),
                    };
                    match outputs.get_mut(name.as_str()) {
                        Some(slot) => *slot = value,
                        None => {
                            outputs.insert(name.clone(), value);
                        }
                    }
                }
            }
        }
        Ok(())
    }
    #[cfg(unix)]
    fn exchange(&mut self, step: CosimStep) -> SimResult<CosimStepResult> {
        let deadline = Instant::now() + self.options.timeout;
        // Preflight borrows caller text; no serde tree/string clone is made
        // before rejecting excessive or nonfinite scalar inputs.
        let mut raw_bytes = 0usize;
        if step.inputs.len() > self.options.max_request_bytes / 4 {
            return Err(error("too many co-sim inputs"));
        }
        for (key, value) in &step.inputs {
            raw_bytes = raw_bytes.saturating_add(key.len());
            match value {
                CosimSignalValue::Text(text) => raw_bytes = raw_bytes.saturating_add(text.len()),
                CosimSignalValue::F64(value) if !value.is_finite() => {
                    return Err(error("nonfinite co-sim input"))
                }
                _ => {}
            }
            if raw_bytes > self.options.max_request_bytes {
                return Err(error("co-sim request exceeds byte limit"));
            }
        }
        let mut request = LimitedWriter {
            bytes: Vec::new(),
            limit: self.options.max_request_bytes - 1,
        };
        serde_json::to_writer(&mut request, &step_to_json(step))
            .map_err(|err| error(format!("co-sim request encoding/byte limit: {err}")))?;
        request.bytes.push(b'\n');
        // Reject data already queued before this request, including an extra
        // frame arriving between steps. This does not claim provenance for
        // arbitrary bytes a malicious peer sends after this check/request.
        reject_queued_bytes(
            self.stdout
                .as_mut()
                .ok_or_else(|| error("co-sim process failed"))?,
            deadline,
        )?;
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| error("co-sim process failed"))?;
        let mut written = 0;
        while written < request.bytes.len() {
            ready(stdin.as_raw_fd(), libc::POLLOUT, deadline)?;
            match stdin.write(&request.bytes[written..]) {
                Ok(0) => return Err(error("co-sim stdin closed")),
                Ok(n) => written += n,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(error(format!("co-sim write failed: {e}"))),
            }
        }
        let stdout = self
            .stdout
            .as_mut()
            .ok_or_else(|| error("co-sim process failed"))?;
        let mut response = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            ready(stdout.as_raw_fd(), libc::POLLIN, deadline)?;
            let remaining = self.options.max_response_bytes - response.len();
            let read_len = chunk.len().min(remaining + 1);
            match stdout.read(&mut chunk[..read_len]) {
                Ok(0) => {
                    return Err(error(
                        "co-sim process exited before returning a complete JSONL response",
                    ))
                }
                Ok(n) => {
                    if n > remaining {
                        return Err(error("co-sim response exceeds byte limit"));
                    }
                    if let Some(end) = chunk[..n].iter().position(|byte| *byte == b'\n') {
                        if end + 1 != n {
                            return Err(error("co-sim response has unsolicited trailing bytes"));
                        }
                        response.extend_from_slice(&chunk[..end]);
                        // A frame ending exactly at a read boundary can leave
                        // more bytes in the pipe, outside the current chunk.
                        reject_queued_bytes(stdout, deadline)?;
                        let value = serde_json::from_slice(&response).map_err(|err| {
                            error(format!("failed to decode co-sim response: {err}"))
                        })?;
                        let result = json_to_step_result(value)?;
                        if Instant::now() >= deadline {
                            return Err(error("co-sim exchange deadline exceeded"));
                        }
                        return Ok(result);
                    }
                    response.extend_from_slice(&chunk[..n]);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(error(format!("co-sim read failed: {e}"))),
            }
        }
    }
}
impl Drop for ExternalProcessCosimAdapter {
    fn drop(&mut self) {
        self.cleanup();
    }
}
impl CosimAdapter for ExternalProcessCosimAdapter {
    fn step_into(
        &mut self,
        time_ns: u64,
        dt_ns: u64,
        inputs: &CosimSignals,
        outputs: &mut CosimSignals,
    ) -> SimResult<()> {
        #[cfg(unix)]
        if self.shm.is_some() {
            if self.failed {
                return Err(error("co-sim adapter is terminally failed; no restart"));
            }
            let result = self.shm_exchange(time_ns, dt_ns, inputs, outputs);
            if result.is_err() {
                self.cleanup();
            }
            return result;
        }
        let result = self.step(CosimStep {
            time_ns,
            dt_ns,
            inputs: inputs.clone(),
        })?;
        *outputs = result.outputs;
        Ok(())
    }

    fn step(&mut self, step: CosimStep) -> SimResult<CosimStepResult> {
        if self.failed {
            return Err(error("co-sim adapter is terminally failed; no restart"));
        }
        #[cfg(unix)]
        if self.shm.is_some() {
            let mut outputs = CosimSignals::new();
            self.step_into(step.time_ns, step.dt_ns, &step.inputs, &mut outputs)?;
            return Ok(CosimStepResult { outputs });
        }
        #[cfg(unix)]
        {
            let result = self.exchange(step);
            if result.is_err() {
                self.cleanup();
            }
            result
        }
        #[cfg(not(unix))]
        {
            let _ = (step, self.options);
            Err(SimulationError::NotImplemented(
                "external-process co-simulation requires native Unix".into(),
            ))
        }
    }
}
#[cfg(unix)]
fn reject_queued_bytes(stdout: &mut ChildStdout, deadline: Instant) -> SimResult<()> {
    let mut byte = [0u8; 1];
    loop {
        if Instant::now() >= deadline {
            return Err(error("co-sim exchange deadline exceeded"));
        }
        match stdout.read(&mut byte) {
            Ok(0) => return Ok(()), // EOF does not invalidate a complete frame.
            Ok(_) => return Err(error("co-sim response has unsolicited queued bytes")),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(error(format!("co-sim queued-byte check failed: {e}"))),
        }
    }
}
#[cfg(unix)]
fn nonblocking(fd: i32) -> SimResult<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(error(format!(
            "co-sim nonblocking pipe: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}
#[cfg(unix)]
fn ready(fd: i32, events: i16, deadline: Instant) -> SimResult<()> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| error("co-sim exchange deadline exceeded"))?;
        let millis = remaining.as_millis().max(1).min(i32::MAX as u128) as i32;
        let mut descriptor = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut descriptor, 1, millis) };
        if result > 0 {
            if descriptor.revents & libc::POLLNVAL != 0 {
                return Err(error("invalid co-sim pipe"));
            }
            // HUP/ERR still allow read()/write() to report EOF/broken pipe.
            return Ok(());
        }
        if result < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err(error(format!(
                "co-sim poll failed: {}",
                std::io::Error::last_os_error()
            )));
        }
    }
}
#[cfg(unix)]
struct LimitedWriter {
    bytes: Vec<u8>,
    limit: usize,
}
#[cfg(unix)]
impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("co-sim request exceeds byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[cfg(unix)]
fn step_to_json(step: CosimStep) -> Value {
    let mut root = Map::new();
    root.insert("time_ns".to_string(), Value::from(step.time_ns));
    root.insert("dt_ns".to_string(), Value::from(step.dt_ns));
    root.insert("inputs".to_string(), signals_to_json(step.inputs));
    Value::Object(root)
}

#[cfg(unix)]
fn signals_to_json(signals: CosimSignals) -> Value {
    let mut map = Map::new();
    for (key, value) in signals {
        map.insert(key, signal_to_json(value));
    }
    Value::Object(map)
}

#[cfg(unix)]
fn signal_to_json(value: CosimSignalValue) -> Value {
    match value {
        CosimSignalValue::Bool(value) => Value::Bool(value),
        CosimSignalValue::I64(value) => Value::from(value),
        CosimSignalValue::F64(value) => Value::from(value),
        CosimSignalValue::Text(value) => Value::String(value),
    }
}

#[cfg(unix)]
fn json_to_step_result(value: Value) -> SimResult<CosimStepResult> {
    let object = value.as_object().ok_or_else(|| {
        SimulationError::Other("co-sim response must be a JSON object".to_string())
    })?;
    let outputs = object
        .get("outputs")
        .and_then(|value| value.as_object())
        .ok_or_else(|| {
            SimulationError::Other(
                "co-sim response must contain object field 'outputs'".to_string(),
            )
        })?;

    let mut result = CosimSignals::new();
    for (key, value) in outputs {
        result.insert(key.clone(), json_to_signal(value)?);
    }

    Ok(CosimStepResult { outputs: result })
}

#[cfg(unix)]
fn json_to_signal(value: &Value) -> SimResult<CosimSignalValue> {
    match value {
        Value::Bool(value) => Ok(CosimSignalValue::Bool(*value)),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Ok(CosimSignalValue::I64(value))
            } else if let Some(value) = value.as_f64() {
                Ok(CosimSignalValue::F64(value))
            } else {
                Err(SimulationError::Other(format!(
                    "unsupported co-sim numeric value '{value}'"
                )))
            }
        }
        Value::String(value) => Ok(CosimSignalValue::Text(value.clone())),
        _ => Err(SimulationError::Other(format!(
            "unsupported co-sim signal value '{value}'"
        ))),
    }
}
