use crate::{all_tests::create_test_elf, TEMPORARY_DIR};
use assert_cmd::cargo_bin_cmd;
use predicates::prelude::{predicate, PredicateBooleanExt};
use scry_asm::{Assemble, Raw};
use std::{io::Write, iter::once, time::Duration};

/// Tests can give input value relative to the build-in "entry" label.
#[test]
fn input_entry_offset()
{
	let program = "\
		add =>1ret 0";
	let assembly = Raw::assemble(once(program)).unwrap();

	let test = |entry_addr, offset| {
		let elf = create_test_elf(assembly.as_slice(), entry_addr, false);
		let mut elf_bytes = Vec::new();
		elf.write(&mut elf_bytes).unwrap();
		std::fs::create_dir_all(TEMPORARY_DIR).unwrap();
		let file = tempfile::Builder::new().tempfile_in(TEMPORARY_DIR).unwrap();
		file.as_file().write_all(elf_bytes.as_slice()).unwrap();

		// Run on the file with relative input
		let mut cmd = cargo_bin_cmd!("scryer");
		cmd.arg(file.path());
		cmd.arg("--machine-mode");
		cmd.arg("--target=scry32-unknown-none-elf");
		cmd.arg(format!("-i=entry+{}i8", offset));

		// Check Results
		let assert = cmd.timeout(Duration::new(5, 0)).assert();
		assert
			.code(predicate::eq((entry_addr + offset + 1) as i32))
			.stdout(predicates::str::is_empty())
			.stderr(predicates::str::is_empty());
	};

	test(10, 0);
	test(100, 3);
	test(150, 100);
}

/// Tests that '--print-events' writes the simulation events to the given file.
#[test]
fn print_events_to_file()
{
	// A program that runs one instruction and returns
	let program = "\
		add =>1ret 0";
	std::fs::create_dir_all(TEMPORARY_DIR).unwrap();
	let program_file = tempfile::Builder::new().tempfile_in(TEMPORARY_DIR).unwrap();
	program_file
		.as_file()
		.write_all(program.as_bytes())
		.unwrap();
	let events_file = tempfile::Builder::new().tempfile_in(TEMPORARY_DIR).unwrap();

	let mut cmd = cargo_bin_cmd!("scryer");
	cmd.arg(program_file.path())
		.arg("--target=assembly")
		.arg("-i=1u8")
		.arg(format!(
			"--print-events={}",
			events_file.path().to_str().unwrap()
		));

	// Events must not be printed to stdout or stderr
	cmd.timeout(Duration::new(5, 0))
		.assert()
		.success()
		.stdout(predicate::str::contains("Returned Operands"))
		.stdout(predicate::str::contains("STEP_END").not())
		.stderr(predicates::str::is_empty());

	// Events must be in the file
	let events = std::fs::read_to_string(events_file.path()).unwrap();
	assert!(dbg!(&events).contains("Instruction: (0x0) add =>1"));
	assert!(events.contains("Instruction: (0x2) ret 0"));
	assert_eq!(events.matches("STEP_END").count(), 2);
}

/// Tests that '--print-events' writes the simulation events to stdout when not
/// given a path
#[test]
fn print_events_to_stdout()
{
	// A program that runs one instruction and returns
	let program = "\
		add =>1ret 0";
	std::fs::create_dir_all(TEMPORARY_DIR).unwrap();
	let program_file = tempfile::Builder::new().tempfile_in(TEMPORARY_DIR).unwrap();
	program_file
		.as_file()
		.write_all(program.as_bytes())
		.unwrap();

	let mut cmd = cargo_bin_cmd!("scryer");
	cmd.arg(program_file.path())
		.arg("--target=assembly")
		.arg("-i=1u8")
		.arg("--print-events");

	// Events must not be printed to stdout or stderr
	cmd.timeout(Duration::new(5, 0))
		.assert()
		.success()
		.stdout(predicate::str::contains("Returned Operands"))
		.stdout(predicate::str::contains("Instruction: (0x0) add =>1"))
		.stdout(predicate::str::contains("Instruction: (0x2) ret 0"))
		.stdout(predicate::str::contains("STEP_END").count(2))
		.stderr(predicates::str::is_empty());
}
