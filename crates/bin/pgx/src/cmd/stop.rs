#![allow(unused)]
use crate::errors::{PgxError, Result};
use crate::out::Out;
pub fn run(_out: &Out, _force: bool) -> Result<()> { Err(PgxError::failure("stop: not implemented yet")) }
