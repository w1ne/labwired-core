use crate::{Execution, PhysicalBus};
use libloading::os::unix::{Library, RTLD_LOCAL, RTLD_NOW};
use std::{
    cell::Cell,
    ffi::{c_char, c_void, CStr},
    marker::PhantomData,
    panic::{catch_unwind, AssertUnwindSafe},
};
use tempfile::TempDir;
type Callback = unsafe extern "C" fn(*mut c_void, u32, u64, u64, u64, u64, *mut u64) -> i32;
type Bind = unsafe extern "C" fn(*mut c_void, Option<Callback>);
type Call = unsafe extern "C" fn(u32, u64, u64, *mut u64, *mut u64) -> i32;
type Error = unsafe extern "C" fn() -> *const c_char;
struct Context<'a> {
    bus: &'a mut dyn PhysicalBus,
    error: Option<String>,
}
// This function NEVER calls native code or unwinds. C observes its return code
// only after the Rust stack frame and its panic payload are gone.
unsafe extern "C" fn callback(
    p: *mut c_void,
    kind: u32,
    a: u64,
    b: u64,
    s: u64,
    v: u64,
    out: *mut u64,
) -> i32 {
    let ctx = &mut *(p as *mut Context<'_>);
    if ctx.error.is_some() {
        return -1;
    }
    let result = catch_unwind(AssertUnwindSafe(|| match kind {
        0 => ctx.bus.read(a, b as u32, s),
        1 => ctx.bus.write(a, b as u32, v, s).map(|()| 0),
        2 => ctx.bus.cp15_read(a as u32, b != 0),
        3 => ctx.bus.cp15_write(a as u32, b != 0, v).map(|()| 0),
        _ => Err("unknown native callback".into()),
    }));
    match result {
        Ok(Ok(value)) => {
            *out = value;
            0
        }
        Ok(Err(e)) => {
            ctx.error = Some(e);
            -1
        }
        Err(payload) => {
            // A hostile panic payload may panic on Drop. Forget it at the FFI boundary.
            std::mem::forget(payload);
            ctx.error = Some("physical bus callback panicked".into());
            -1
        }
    }
}
struct Binding {
    bind: Bind,
}
impl Drop for Binding {
    fn drop(&mut self) {
        unsafe { (self.bind)(std::ptr::null_mut(), None) }
    }
}
/// A CPU can be moved while idle. It is deliberately not Sync.
pub struct ArmCpu {
    // Drop native library before deleting the file containing it.
    library: Library,
    directory: TempDir,
    bind: Bind,
    call: Call,
    error: Error,
    initialized: bool,
    page_size: u64,
    poisoned: bool,
    ranges: Vec<(u64, u64)>,
    bus_identity: Option<u64>,
    not_sync: PhantomData<Cell<()>>,
}
impl ArmCpu {
    pub fn new() -> Result<Self, String> {
        let directory = tempfile::Builder::new()
            .prefix("labwired-arm-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let path = directory.path().join("libarm.so");
        std::fs::write(&path, include_bytes!(env!("LABWIRED_ARM_SYSTEM_LIBRARY")))
            .map_err(|e| e.to_string())?;
        // A distinct file inode is required: dlopen caches handles by inode, so
        // loading the same file twice would share mutable CPU and TCG globals.
        let library = unsafe { Library::open(Some(&path), RTLD_NOW | RTLD_LOCAL) }
            .map_err(|e| e.to_string())?;
        let (bind, call, error) = unsafe {
            (
                *library
                    .get::<Bind>(b"lw_bind\0")
                    .map_err(|e| e.to_string())?,
                *library
                    .get::<Call>(b"lw_call\0")
                    .map_err(|e| e.to_string())?,
                *library
                    .get::<Error>(b"lw_error\0")
                    .map_err(|e| e.to_string())?,
            )
        };
        let mut cpu = Self {
            library,
            directory,
            bind,
            call,
            error,
            initialized: false,
            page_size: 0,
            poisoned: false,
            ranges: vec![],
            bus_identity: None,
            not_sync: PhantomData,
        };
        cpu.invoke(0, 0, 0)?;
        cpu.initialized = true;
        cpu.page_size = cpu.invoke(9, 0, 0)?.0;
        if !cpu.page_size.is_power_of_two() {
            return Err("native page size must be a nonzero power of two".into());
        }
        Ok(cpu)
    }
    fn invoke(&mut self, op: u32, a: u64, b: u64) -> Result<(u64, u64), String> {
        if self.poisoned {
            return Err("ARM CPU is poisoned; reset or recreate it".into());
        }
        let (mut out, mut extra) = (0, 0);
        if unsafe { (self.call)(op, a, b, &mut out, &mut extra) } != 0 {
            self.poisoned = true;
            return Err(unsafe { CStr::from_ptr((self.error)()) }
                .to_string_lossy()
                .into_owned());
        }
        Ok((out, extra))
    }
    fn register_number(n: u8) -> Result<u64, String> {
        match n {
            0..=15 => Ok(n as u64),
            16 => Ok(25),
            _ => Err(format!(
                "unsupported ARM register {n}; use 0..=15 or 16 for CPSR"
            )),
        }
    }
    pub fn register(&mut self, n: u8) -> Result<u32, String> {
        Ok(self.invoke(2, Self::register_number(n)?, 0)?.0 as u32)
    }
    pub fn set_register(&mut self, n: u8, v: u32) -> Result<(), String> {
        self.invoke(3, Self::register_number(n)?, v as u64)
            .map(|_| ())
    }
    pub fn executable_range(&mut self, start: u64, length: u64) -> Result<(), String> {
        if length == 0 || start.checked_add(length).is_none_or(|end| end > 1u64 << 32) {
            return Err(
                "executable range must be nonempty and inside the 32-bit physical address space"
                    .into(),
            );
        }
        let end = start + length;
        let rounded_start = start & !(self.page_size - 1);
        let rounded_end = (end + self.page_size - 1) & !(self.page_size - 1);
        let rounded_length = rounded_end - rounded_start;
        self.invoke(4, rounded_start, rounded_length)?;
        self.ranges.push((rounded_start, rounded_length));
        Ok(())
    }
    pub fn interrupt_levels(&mut self, irq: bool, fiq: bool) -> Result<(), String> {
        self.invoke(5, irq as u64, fiq as u64).map(|_| ())
    }
    pub fn execute(&mut self, bus: &mut dyn PhysicalBus, count: u32) -> Result<Execution, String> {
        if count == 0 {
            return Err("execution budget must be nonzero".into());
        }
        // Allocation-unique identities avoid stale code even when bus storage is reused.
        let identity = bus.code_identity();
        if self.bus_identity != Some(identity) {
            self.invalidate_code()?;
            self.invoke(11, 0, 0)?;
            self.bus_identity = Some(identity);
        }
        let mut ctx = Context { bus, error: None };
        unsafe { (self.bind)(&mut ctx as *mut _ as *mut c_void, Some(callback)) };
        let binding = Binding { bind: self.bind };
        let result = self.invoke(6, count as u64, 0);
        drop(binding);
        if let Some(error) = ctx.error {
            self.poisoned = true;
            return Err(error);
        }
        result.map(|(retired, waiting)| Execution {
            retired,
            waiting: waiting != 0,
        })
    }
    pub fn invalidate_code(&mut self) -> Result<(), String> {
        self.invoke(7, 0, 0).map(|_| ())
    }
    pub fn reset(&mut self) -> Result<(), String> {
        // Recreate after faults rather than reuse partially unwound translator state.
        let mut replacement = Self::new()?;
        for &(start, length) in &self.ranges {
            replacement.executable_range(start, length)?;
        }
        *self = replacement;
        Ok(())
    }
}
impl Drop for ArmCpu {
    fn drop(&mut self) {
        unsafe { (self.bind)(std::ptr::null_mut(), None) };
        let (mut a, mut b) = (0, 0);
        // Disposal also uses the C fatal guard, including for a poisoned CPU.
        if self.initialized {
            unsafe { (self.call)(8, 0, 0, &mut a, &mut b) };
        }
        // Keep these ownership fields visibly live through native disposal.
        let _ = (&self.library, &self.directory);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;
    #[test]
    fn instances_have_distinct_inodes() {
        let a = ArmCpu::new().unwrap();
        let b = ArmCpu::new().unwrap();
        let ma = std::fs::metadata(a.directory.path().join("libarm.so")).unwrap();
        let mb = std::fs::metadata(b.directory.path().join("libarm.so")).unwrap();
        assert_ne!((ma.dev(), ma.ino()), (mb.dev(), mb.ino()));
    }
    #[test]
    fn native_fatal_register_access_is_contained() {
        let mut c = ArmCpu::new().unwrap();
        assert!(c
            .invoke(2, 999, 0)
            .unwrap_err()
            .contains("undefined CPU register"));
        assert!(c.register(0).is_err());
        c.reset().unwrap();
        assert_eq!(c.register(0).unwrap(), 0);
    }
    #[test]
    fn native_fatal_dispatch_is_contained() {
        let mut c = ArmCpu::new().unwrap();
        assert!(c
            .invoke(99, 0, 0)
            .unwrap_err()
            .contains("unknown bridge operation"));
        c.reset().unwrap();
        assert_eq!(c.register(0).unwrap(), 0);
    }
    #[test]
    fn direct_native_abort_and_assert_are_contained() {
        if let Ok(kind) = std::env::var("LABWIRED_NATIVE_FATAL_PROBE") {
            let mut c = ArmCpu::new().unwrap();
            let message = c.invoke(10, kind.parse().unwrap(), 0).unwrap_err();
            assert!(
                message.contains(if kind == "0" {
                    "native abort"
                } else {
                    "native assertion"
                }),
                "{message}"
            );
            return;
        }
        for kind in ["0", "1"] {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "native::tests::direct_native_abort_and_assert_are_contained",
                ])
                .env("LABWIRED_NATIVE_FATAL_PROBE", kind)
                .status()
                .unwrap();
            assert!(
                status.success(),
                "native fatal probe {kind} exited {status}"
            );
        }
    }
}
