use std::fmt;

/// Everything that can go wrong while reading or writing a WSQ stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The stream ended before a complete structure could be read.
    Truncated,
    /// A marker was expected at `offset` but the bytes there are not one.
    ExpectedMarker { offset: usize },
    /// A marker that is not allowed at this point of the stream.
    UnexpectedMarker { offset: usize, marker: u16 },
    /// A marker segment whose content is inconsistent.
    BadSegment(&'static str),
    /// A table needed for decoding was never defined.
    MissingTable(&'static str),
    /// A Huffman table that cannot be turned into a prefix code.
    BadHuffmanTable,
    /// A bit sequence that is not a code of the current table.
    BadHuffmanCode,
    /// A Huffman symbol outside the WSQ alphabet.
    BadSymbol(u8),
    /// The entropy-coded data did not yield the expected number of indices.
    CoefficientCount { expected: usize, found: usize },
    /// The image is larger than the decoder was allowed to allocate.
    TooLarge { width: u32, height: u32 },
    /// Zero width or height, or a pixel buffer of the wrong length.
    BadDimensions,
    /// An encoder parameter out of range.
    BadParameter(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated => write!(f, "WSQ stream is truncated"),
            Error::ExpectedMarker { offset } => write!(f, "expected a marker at byte {offset}"),
            Error::UnexpectedMarker { offset, marker } => {
                write!(f, "unexpected marker {marker:#06X} at byte {offset}")
            }
            Error::BadSegment(what) => write!(f, "invalid marker segment: {what}"),
            Error::MissingTable(what) => write!(f, "missing table: {what}"),
            Error::BadHuffmanTable => write!(f, "invalid Huffman table"),
            Error::BadHuffmanCode => write!(f, "invalid Huffman code in entropy-coded data"),
            Error::BadSymbol(s) => write!(f, "invalid Huffman symbol {s}"),
            Error::CoefficientCount { expected, found } => {
                write!(f, "expected {expected} quantizer indices, decoded {found}")
            }
            Error::TooLarge { width, height } => {
                write!(f, "image of {width}x{height} pixels exceeds the decoder's limit")
            }
            Error::BadDimensions => write!(f, "invalid image dimensions"),
            Error::BadParameter(what) => write!(f, "invalid parameter: {what}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
