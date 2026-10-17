#![allow(unused)]
use crate::cli::ExitCode;
use crate::errors::{PgxError, Result};
use crate::out::Out;
pub fn run(_out: &Out) -> Result<ExitCode> { Err(PgxError::failure("doctor: not implemented yet")) }
