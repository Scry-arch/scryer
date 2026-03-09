use crate::TEMPORARY_DIR;
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::predicate;
use scry_sim::Metric;
use std::{
	io::Write,
	time::{Duration, Instant},
};
use tempfile::NamedTempFile;

fn temp_infinite_loop_program() -> NamedTempFile
{
	// Output program to a file
	std::fs::create_dir_all(TEMPORARY_DIR).unwrap();
	let file = tempfile::Builder::new().tempfile_in(TEMPORARY_DIR).unwrap();

	// We use a program that loops forever (assuming no inputs are given).
	file.as_file().write_all("jmp 0,0".as_bytes()).unwrap();
	file
}

/// Tests can ask for a timeout after some number of instructions executed
fn timeout_instructions(instr_timeout: u16)
{
	let file = temp_infinite_loop_program();

	let mut cmd = cargo_bin_cmd!("scryer");
	cmd.arg(file.path())
		.arg(format!("--timeout={}", instr_timeout.to_string()))
		.arg("--timeout-type=instructions")
		.arg("--target=assembly");

	// Test that a timeout is messaged
	let timeout_msg = r"Timeout(.)*?\n(.|\n)*?".to_owned();
	// Test that the metrics show the right number of instructions were executed
	let instr_count = format!(
		r"{:?}(\s)*?:(\s)*?{}\n(.|\n)*?",
		Metric::InstructionReads,
		instr_timeout
	);

	cmd.timeout(Duration::new(60, 0))
		.assert()
		.failure()
		.stdout(predicate::str::is_match(timeout_msg + instr_count.as_str()).unwrap());
}

/// Tests can ask for a timeout after some number of seconds
fn timeout_second(second_timeout: u16, machine_mode: bool)
{
	let file = temp_infinite_loop_program();

	let mut cmd = cargo_bin_cmd!("scryer");
	cmd.arg(file.path())
		.arg(format!("--timeout={}", second_timeout.to_string()))
		.arg("--timeout-type=seconds")
		.arg("--target=assembly");
	if machine_mode
	{
		cmd.arg("--machine-mode");
	}

	// Test that a timeout is messaged
	let timeout_msg = r"Timeout(.)*?\n(.|\n)*?".to_owned();

	let before = Instant::now();
	let assert = cmd.timeout(Duration::new(60, 0)).assert();
	let elapsed = before.elapsed().as_secs();

	// test timeout with a uncertainty of 2 second
	assert!(elapsed < (second_timeout + 2) as u64);
	assert!(elapsed >= second_timeout as u64);

	let assert = assert.failure();
	if !machine_mode
	{
		assert.stdout(predicate::str::is_match(timeout_msg).unwrap());
	}
}

/// Tests that when the timeout is 0, doesn't time out regardless of type.
/// The given timeout is how long the test should allow the program to run,
/// then check that it ran for that long without finishing.
fn no_timeout(test_timeout: u64, timeout_type: &'static str, machine_mode: bool)
{
	let file = temp_infinite_loop_program();

	let mut cmd = cargo_bin_cmd!("scryer");
	cmd.arg(file.path())
		.arg("--timeout=0")
		.arg("--timeout-type=".to_owned() + timeout_type)
		.arg("--target=assembly");
	if !machine_mode
	{
		cmd.arg("--machine-mode");
	}

	let before = Instant::now();
	let assert = cmd.timeout(Duration::new(test_timeout, 0)).assert();
	let elapsed = before.elapsed().as_secs();

	// test timeout with a uncertainty of 2 second
	assert!(elapsed < test_timeout + 2);
	assert!(elapsed >= test_timeout);

	assert
		.failure()
		.stdout(predicate::str::is_empty())
		.stderr(predicate::str::is_empty());
}

// Test a few different timeouts
#[test]
fn timeout_instructions_100()
{
	timeout_instructions(100)
}
#[test]
fn timeout_instructions_12345()
{
	timeout_instructions(12345)
}
#[test]
fn timeout_instructions_23576()
{
	timeout_instructions(23576)
}

#[test]
fn timeout_seconds_6()
{
	timeout_second(6, false);
}
#[test]
fn timeout_seconds_11()
{
	timeout_second(11, false);
}
#[test]
fn timeout_seconds_14()
{
	timeout_second(14, false);
}
#[test]
fn timeout_seconds_3_machine()
{
	timeout_second(3, true);
}
#[test]
fn timeout_seconds_8_machine()
{
	timeout_second(8, true);
}
#[test]
fn timeout_seconds_11_machine()
{
	timeout_second(11, true);
}

#[test]
fn no_timeout_instructions_10()
{
	no_timeout(10, "instructions", false);
}
#[test]
fn no_timeout_instructions_15()
{
	no_timeout(15, "instructions", false);
}
#[test]
fn no_timeout_seconds_5()
{
	no_timeout(5, "seconds", false);
}
#[test]
fn no_timeout_seconds_7()
{
	no_timeout(7, "seconds", false);
}
#[test]
fn no_timeout_instructions_10_machine()
{
	no_timeout(10, "instructions", true);
}
#[test]
fn no_timeout_instructions_15_machine()
{
	no_timeout(15, "instructions", true);
}
#[test]
fn no_timeout_seconds_5_machine()
{
	no_timeout(5, "seconds", true);
}
#[test]
fn no_timeout_seconds_7_machine()
{
	no_timeout(7, "seconds", true);
}
