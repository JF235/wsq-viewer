//! C ABI of the wsq codec for WebAssembly hosts (the VS Code extension and the benchmark).
//!
//! Every call that can fail returns 1 on success and 0 on failure; the message of the last
//! failure is at `wsq_error_ptr` / `wsq_error_len` (UTF-8). Buffers passed in are allocated by
//! the host with `wsq_alloc` and released with `wsq_free`. Results live inside the module until
//! the next call of the same kind:
//!
//! - `wsq_decode`: `wsq_width`, `wsq_height`, `wsq_ppi` (0 if unknown) and `wsq_pixels`
//!   (`width * height` grey bytes, row-major);
//! - `wsq_info`: a JSON object at `wsq_result_ptr` / `wsq_result_len`;
//! - `wsq_encode`: the WSQ stream at `wsq_result_ptr` / `wsq_result_len`.

use std::cell::RefCell;

#[derive(Default)]
struct State {
    exact: Option<wsq::Decoder>,
    fast: Option<wsq::Decoder>,
    encoder: Option<wsq::Encoder>,
    image: Option<wsq::Image>,
    result: Vec<u8>,
    error: String,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

/// Record the outcome of a call: 1 on success, 0 (with the error message kept) on failure.
fn status<T>(s: &mut State, r: wsq::Result<T>, ok: impl FnOnce(&mut State, T)) -> u32 {
    match r {
        Ok(v) => {
            ok(s, v);
            s.error.clear();
            1
        }
        Err(e) => {
            s.error = e.to_string();
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn wsq_alloc(len: usize) -> *mut u8 {
    let mut v = vec![0u8; len];
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` and `len` must come from one `wsq_alloc` call, released once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wsq_free(ptr: *mut u8, len: usize) {
    unsafe { drop(Vec::from_raw_parts(ptr, len, len)) };
}

/// Decode a stream; `fast` != 0 selects fast mode, otherwise the output is NBIS's, bit for bit.
///
/// # Safety
/// `ptr` must point to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wsq_decode(ptr: *const u8, len: usize, fast: u32) -> u32 {
    let data = unsafe { std::slice::from_raw_parts(ptr, len) };
    with_state(|s| {
        let mode = if fast != 0 { wsq::Mode::Fast } else { wsq::Mode::Exact };
        let slot = if fast != 0 { &mut s.fast } else { &mut s.exact };
        let dec = slot.get_or_insert_with(|| wsq::Decoder::new(wsq::DecodeOptions { mode, ..Default::default() }));
        let r = dec.decode(data);
        status(s, r, |s, img| s.image = Some(img))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn wsq_width() -> u32 {
    with_state(|s| s.image.as_ref().map_or(0, |i| i.width))
}

#[unsafe(no_mangle)]
pub extern "C" fn wsq_height() -> u32 {
    with_state(|s| s.image.as_ref().map_or(0, |i| i.height))
}

#[unsafe(no_mangle)]
pub extern "C" fn wsq_ppi() -> u32 {
    with_state(|s| s.image.as_ref().and_then(|i| i.ppi).unwrap_or(0))
}

#[unsafe(no_mangle)]
pub extern "C" fn wsq_pixels() -> *const u8 {
    with_state(|s| s.image.as_ref().map_or(std::ptr::null(), |i| i.pixels.as_ptr()))
}

/// Read the header, tables and comments of a stream without decoding the image.
///
/// # Safety
/// `ptr` must point to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wsq_info(ptr: *const u8, len: usize) -> u32 {
    let data = unsafe { std::slice::from_raw_parts(ptr, len) };
    with_state(|s| {
        let r = wsq::read_info(data);
        status(s, r, |s, info| s.result = info_json(&info).into_bytes())
    })
}

/// Encode `width * height` grey pixels at `bitrate` bits per pixel; `ppi` 0 leaves the
/// resolution unknown (NBIS writes -1); `fast` != 0 selects fast mode.
///
/// # Safety
/// `ptr` must point to `width * height` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wsq_encode(ptr: *const u8, width: u32, height: u32, bitrate: f32, ppi: u32, fast: u32) -> u32 {
    let pixels = unsafe { std::slice::from_raw_parts(ptr, width as usize * height as usize) };
    with_state(|s| {
        let opts = wsq::EncodeOptions {
            bitrate,
            ppi: (ppi != 0).then_some(ppi),
            mode: if fast != 0 { wsq::Mode::Fast } else { wsq::Mode::Exact },
            ..Default::default()
        };
        let r = s.encoder.get_or_insert_with(wsq::Encoder::new).encode(pixels, width, height, &opts);
        status(s, r, |s, out| s.result = out)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn wsq_result_ptr() -> *const u8 {
    with_state(|s| s.result.as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn wsq_result_len() -> usize {
    with_state(|s| s.result.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn wsq_error_ptr() -> *const u8 {
    with_state(|s| s.error.as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn wsq_error_len() -> usize {
    with_state(|s| s.error.len())
}

/// Vector width of the backend this build runs (1 without SIMD, 4 with simd128).
#[unsafe(no_mangle)]
pub extern "C" fn wsq_lanes() -> u32 {
    wsq::Backend::best().lanes() as u32
}

fn json_string(out: &mut String, bytes: &[u8]) {
    out.push('"');
    for c in String::from_utf8_lossy(bytes).chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn info_json(i: &wsq::Info) -> String {
    let opt = |v: Option<String>| v.unwrap_or_else(|| "null".into());
    let mut comments = String::from("[");
    for (k, c) in i.comments.iter().enumerate() {
        if k > 0 {
            comments.push(',');
        }
        json_string(&mut comments, c);
    }
    comments.push(']');
    format!(
        "{{\"width\":{},\"height\":{},\"ppi\":{},\"bitrate\":{},\"encoder\":{},\"software\":{},\"filters\":[{},{}],\"blocks\":{},\"comments\":{}}}",
        i.width,
        i.height,
        opt(i.ppi.map(|p| p.to_string())),
        opt(i.bitrate.filter(|b| b.is_finite()).map(|b| b.to_string())),
        i.encoder,
        i.software,
        i.filters.0,
        i.filters.1,
        i.blocks,
        comments
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result() -> Vec<u8> {
        unsafe { std::slice::from_raw_parts(wsq_result_ptr(), wsq_result_len()) }.to_vec()
    }

    fn error() -> String {
        String::from_utf8(unsafe { std::slice::from_raw_parts(wsq_error_ptr(), wsq_error_len()) }.to_vec()).unwrap()
    }

    fn ridges(w: u32, h: u32) -> Vec<u8> {
        (0..h).flat_map(|y| (0..w).map(move |x| (128.0 + 90.0 * ((x as f32 * 0.31 + y as f32 * 0.17).sin())) as u8)).collect()
    }

    #[test]
    fn encode_info_decode_round_trip() {
        let (w, h) = (300u32, 200u32);
        let px = ridges(w, h);
        assert_eq!(unsafe { wsq_encode(px.as_ptr(), w, h, 0.75, 500, 0) }, 1);
        let stream = result();

        assert_eq!(unsafe { wsq_info(stream.as_ptr(), stream.len()) }, 1);
        let json = String::from_utf8(result()).unwrap();
        assert!(json.starts_with("{\"width\":300,\"height\":200,\"ppi\":500,\"bitrate\":0.75,"), "{json}");
        assert!(json.contains("\"filters\":[9,7]") && json.contains("NIST_COM"), "{json}");

        for fast in [0, 1] {
            assert_eq!(unsafe { wsq_decode(stream.as_ptr(), stream.len(), fast) }, 1);
            assert_eq!((wsq_width(), wsq_height(), wsq_ppi()), (w, h, 500));
            let out = unsafe { std::slice::from_raw_parts(wsq_pixels(), (w * h) as usize) };
            let mse = out.iter().zip(&px).map(|(&a, &b)| (a as f64 - b as f64).powi(2)).sum::<f64>() / px.len() as f64;
            assert!(mse < 40.0, "mse {mse}");
        }
    }

    #[test]
    fn failures_report_a_message() {
        let junk = [0xffu8, 0xa0, 0x00, 0x01];
        assert_eq!(unsafe { wsq_decode(junk.as_ptr(), junk.len(), 0) }, 0);
        assert!(!error().is_empty());
        assert_eq!(unsafe { wsq_info(junk.as_ptr(), 2) }, 0);
        assert!(!error().is_empty());
    }

    #[test]
    fn comments_are_escaped() {
        let mut s = String::new();
        json_string(&mut s, b"a\"b\\c\nd\x01");
        assert_eq!(s, "\"a\\\"b\\\\c\\nd\\u0001\"");
    }
}
