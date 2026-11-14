#![no_main]
use libfuzzer_sys::fuzz_target;

#[cfg(target_os = "linux")]
fuzz_target!(|data: &[u8]| {
    let input = String::from_utf8_lossy(data);
    let _ = freshen_lib::apt::parse_apt_simulated_upgrade(&input);
});
