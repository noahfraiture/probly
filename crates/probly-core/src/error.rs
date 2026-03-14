use core::fmt;

pub type Result<T, E = Error> = core::result::Result<T, E>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    PrecisionMismatch { left: u8, right: u8 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PrecisionMismatch { left, right } => {
                write!(
                    f,
                    "cannot merge sketches with different precision: left={left}, right={right}"
                )
            }
        }
    }
}

impl std::error::Error for Error {}
