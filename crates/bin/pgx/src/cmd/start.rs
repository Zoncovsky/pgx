#![allow(unused)]
use crate::errors::{PgxError, Result};
use crate::out::Out;
pub fn run(_out: &Out) -> Result<()> { Err(PgxError::failure("start: not implemented yet")) }
pub fn restart(_out: &Out) -> Result<()> { Err(PgxError::failure("restart: not implemented yet")) }
