//! On Linux the main thread's stack grows on demand up to RLIMIT_STACK, so the guest check judges
//! it by that limit, not by what the C library reports for the pages mapped so far (musl reports
//! only those).
fn main() {
    #[cfg(target_os = "linux")]
    {
        let required =
            (sunrise_guest_stack::GUEST_STACK_BYTES + sunrise_guest_stack::HOST_HEADROOM) as u64;
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: limit is writable and getrlimit initializes it on success.
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut limit) },
            0
        );
        if limit.rlim_max < required {
            println!("skipped: the stack hard limit is below the guest's reservation");
            return;
        }
        limit.rlim_cur = required;
        // SAFETY: limit is initialized; this changes only this process's resource limit.
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_STACK, &limit) }, 0);
        assert_eq!(sunrise_guest_stack::check_current(), Ok(()));
    }
}
