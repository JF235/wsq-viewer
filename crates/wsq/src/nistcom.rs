//! The NISTCOM comment convention of NBIS: "NIST_COM n" followed by "KEY value" lines.

/// Text of the comment NBIS's encoder writes (identical bytes).
pub fn nbis_comment(width: usize, height: usize, ppi: Option<u32>, bitrate: f32) -> String {
    let ppi = ppi.map(|p| p as i64).unwrap_or(-1);
    format!(
        "NIST_COM 9\nPIX_WIDTH {width}\nPIX_HEIGHT {height}\nPIX_DEPTH 8\nPPI {ppi}\nLOSSY 1\n\
         COLORSPACE GRAY\nCOMPRESSION WSQ\nWSQ_BITRATE {:.6}",
        bitrate as f64
    )
}

/// Value of `key` in a NISTCOM comment, if the comment is one and has the key.
pub fn lookup<'a>(comment: &'a [u8], key: &str) -> Option<&'a str> {
    let text = std::str::from_utf8(comment).ok()?;
    let text = text.trim_end_matches('\0');
    if !text.starts_with("NIST_COM") {
        return None;
    }
    text.lines().find_map(|line| {
        let (k, v) = line.split_once(' ')?;
        (k == key).then(|| v.trim())
    })
}

/// Resolution from the NISTCOM comments of a stream.
pub fn ppi(comments: &[Vec<u8>]) -> Option<u32> {
    comments.iter().find_map(|c| lookup(c, "PPI")?.parse::<i64>().ok()).filter(|&p| p > 0).map(|p| p as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_nbis_bytes() {
        let s = nbis_comment(1452, 1006, Some(500), 0.75);
        assert_eq!(
            s,
            "NIST_COM 9\nPIX_WIDTH 1452\nPIX_HEIGHT 1006\nPIX_DEPTH 8\nPPI 500\nLOSSY 1\nCOLORSPACE GRAY\nCOMPRESSION WSQ\nWSQ_BITRATE 0.750000"
        );
        assert_eq!(lookup(s.as_bytes(), "PPI"), Some("500"));
        assert_eq!(ppi(&[s.into_bytes()]), Some(500));
    }
}
