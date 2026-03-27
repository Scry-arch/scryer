#![cfg_attr(feature = "fail-on-warnings", deny(warnings))]

use clap::Parser;
use scryer::Cli;
use std::{
	io::{stderr, stdout},
	process::exit,
};

fn main()
{
	let exit_code = scryer::run(Cli::parse(), stdout(), stderr());
	exit(exit_code);
}
