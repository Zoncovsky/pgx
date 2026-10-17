#![allow(unused)]
use crate::errors::{PgxError, Result};
use crate::out::Out;
pub fn create(_out: &Out, _name: &str) -> Result<()> { Err(PgxError::failure("not implemented yet")) }
pub fn drop(_out: &Out, _name: &str, _force: bool) -> Result<()> { Err(PgxError::failure("not implemented yet")) }
pub fn clone(_out: &Out, _s: &str, _t: &str) -> Result<()> { Err(PgxError::failure("not implemented yet")) }
pub fn ls(_out: &Out) -> Result<()> { Err(PgxError::failure("not implemented yet")) }
pub fn url(_out: &Out, _name: &str, _socket: bool) -> Result<()> { Err(PgxError::failure("not implemented yet")) }
