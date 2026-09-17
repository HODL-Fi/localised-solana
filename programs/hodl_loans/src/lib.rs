use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;

pub use constants::*;
pub use errors::*;

declare_id!("J9sKAhm2EhdJQ3bHeP2KUCxqZ4cYdBc65C3RDr4JjGEd");

#[program]
pub mod hodl_loans {}
