#![no_main]

use hl_capture::spool::validate_segment_bytes;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = validate_segment_bytes(input);
});
