//! A library built with `-C panic=abort`. That strategy cannot link the dynamic std the SDK route uses, so it
//! carries its own std and a build identity: the game must refuse it at load.
#[unsafe(no_mangle)]
pub extern "C" fn sunrise_native_build() -> *const std::os::raw::c_char {
    b"abort\0".as_ptr().cast()
}
