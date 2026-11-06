//! A minimal C interface over the codec for WebAssembly hosts (the VS Code extension and the
//! benchmarks). Memory is exchanged through buffers allocated with `wsq_alloc`.

use std::cell::RefCell;

thread_local! {
    static DECODER: RefCell<Option<wsq::Decoder>> = const { RefCell::new(None) };
    static LAST: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Allocate `len` bytes and return their address.
#[unsafe(no_mangle)]
pub extern "C" fn wsq_alloc(len: usize) -> *mut u8 {
    let mut v = vec![0u8; len];
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Free a buffer from `wsq_alloc`.
///
/// # Safety
/// `ptr` and `len` must come from `wsq_alloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wsq_free(ptr: *mut u8, len: usize) {
    unsafe { drop(Vec::from_raw_parts(ptr, len, len)) };
}

/// Decode `len` bytes at `ptr`. Returns `width << 16 | height`, or 0 on error; the pixels are
/// then available through `wsq_pixels`. `fast` selects the fast arithmetic.
///
/// # Safety
/// `ptr` must point to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wsq_decode(ptr: *const u8, len: usize, fast: u32) -> u32 {
    let data = unsafe { std::slice::from_raw_parts(ptr, len) };
    DECODER.with(|d| {
        let mut d = d.borrow_mut();
        let mode = if fast != 0 { wsq::Mode::Fast } else { wsq::Mode::Exact };
        let dec = d.get_or_insert_with(|| wsq::Decoder::new(wsq::DecodeOptions { mode, ..Default::default() }));
        match dec.decode(data) {
            Ok(img) => {
                let dims = (img.width << 16) | img.height;
                LAST.with(|l| *l.borrow_mut() = img.pixels);
                dims
            }
            Err(_) => 0,
        }
    })
}

/// Address of the pixels of the last decoded image.
#[unsafe(no_mangle)]
pub extern "C" fn wsq_pixels() -> *const u8 {
    LAST.with(|l| l.borrow().as_ptr())
}

/// Name of the vector backend this build uses.
#[unsafe(no_mangle)]
pub extern "C" fn wsq_backend() -> u32 {
    wsq::Backend::best().lanes() as u32
}
