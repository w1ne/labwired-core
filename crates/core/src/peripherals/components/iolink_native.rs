#![cfg(feature = "iolink-native")]

unsafe extern "C" {
    fn lw_iolm_backend_name() -> *const std::os::raw::c_char;
}

pub fn backend_name() -> &'static str {
    unsafe {
        let ptr = lw_iolm_backend_name();
        assert!(!ptr.is_null(), "lw_iolm_backend_name returned null");
        std::ffi::CStr::from_ptr(ptr)
            .to_str()
            .expect("backend name must be utf-8")
    }
}
