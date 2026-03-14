use std::hash::Hash;

pub mod error;
pub mod hll;

pub use error::{Error, Result};
pub use hll::Hll;

trait Probly {
    fn new(precision: u8) -> Self;

    fn add(&mut self, value: &[u8]);

    fn add_hash<T: Hash>(&mut self, value: &T);

    fn merge(&mut self, other: &Self) -> Result<()>;

    fn count(&self) -> usize;
}
