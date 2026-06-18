#![cfg(feature = "iolink-native")]

#[repr(C)]
struct NativeConfig {
    pd_in_len: u8,
    pd_out_len: u8,
    m_seq_type: u8,
    min_cycle_time_100us: u8,
    response_timeout_100us: u8,
    com: u8,
}

unsafe extern "C" {
    fn lw_iolm_backend_name() -> *const std::os::raw::c_char;
    fn lw_iolm_context_size() -> usize;
    fn lw_iolm_init(ctx: *mut std::ffi::c_void, config: *const NativeConfig) -> i32;
    fn lw_iolm_tick(ctx: *mut std::ffi::c_void, event: u32, now_100us: u32) -> i32;
    fn lw_iolm_drain_tx(ctx: *mut std::ffi::c_void, out: *mut u8, out_len: usize) -> usize;
    fn lw_iolm_feed_rx(ctx: *mut std::ffi::c_void, data: *const u8, len: usize) -> usize;
    fn lw_iolm_state_name(ctx: *mut std::ffi::c_void) -> *const std::os::raw::c_char;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeTickEvent {
    None = 0,
    CycleDue = 1,
    ResponseTimeout = 2,
}

#[derive(Debug)]
pub struct NativeIolinkMasterPort {
    storage: Vec<u64>,
}

pub fn backend_name() -> &'static str {
    c_str_to_static(unsafe { lw_iolm_backend_name() })
}

impl NativeIolinkMasterPort {
    pub fn new_type2_com3(pd_in_len: u8, pd_out_len: u8) -> Self {
        let bytes = unsafe { lw_iolm_context_size() };
        let words = (bytes + std::mem::size_of::<u64>() - 1) / std::mem::size_of::<u64>();
        let mut storage = vec![0u64; words];
        let config = NativeConfig {
            pd_in_len,
            pd_out_len,
            m_seq_type: 4,
            min_cycle_time_100us: 20,
            response_timeout_100us: 3,
            com: 2,
        };
        let ret = unsafe { lw_iolm_init(storage.as_mut_ptr().cast(), &config) };
        assert_eq!(ret, 0, "lw_iolm_init failed with {ret}");
        Self { storage }
    }

    pub fn tick(&mut self, event: NativeTickEvent, now_100us: u32) -> i32 {
        unsafe { lw_iolm_tick(self.ptr(), event as u32, now_100us) }
    }

    pub fn drain_tx(&mut self) -> Vec<u8> {
        let mut out = vec![0u8; 64];
        let n = unsafe { lw_iolm_drain_tx(self.ptr(), out.as_mut_ptr(), out.len()) };
        out.truncate(n);
        out
    }

    pub fn feed_rx(&mut self, bytes: &[u8]) {
        unsafe {
            let n = lw_iolm_feed_rx(self.ptr(), bytes.as_ptr(), bytes.len());
            assert_eq!(n, bytes.len());
        }
    }

    pub fn state_name(&mut self) -> &'static str {
        c_str_to_static(unsafe { lw_iolm_state_name(self.ptr()) })
    }

    fn ptr(&mut self) -> *mut std::ffi::c_void {
        self.storage.as_mut_ptr().cast()
    }
}

fn c_str_to_static(ptr: *const std::os::raw::c_char) -> &'static str {
    assert!(!ptr.is_null());
    unsafe {
        std::ffi::CStr::from_ptr(ptr)
            .to_str()
            .expect("native string must be utf-8")
    }
}
