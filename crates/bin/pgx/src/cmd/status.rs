#![allow(unused)]
use crate::errors::{PgxError, Result};
use crate::out::Out;
pub fn run(_out: &Out) -> Result<()> { Err(PgxError::failure("status: not implemented yet")) }
