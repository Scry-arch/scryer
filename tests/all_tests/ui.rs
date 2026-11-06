use crate::{
	all_tests::{create_temp_file, create_test_elf},
	TEMPORARY_DIR,
};
use assert_cmd::cargo_bin_cmd;
use object::{
	build::elf::SectionData,
	elf::{EM_X86_64, SHN_ABS, SHT_STRTAB, SHT_SYMTAB, STB_GLOBAL, STT_NOTYPE},
};
use predicates::prelude::{predicate, PredicateBooleanExt};
use scry_asm::{Assemble, Raw};
use std::{io::Write, iter::once, time::Duration};
use tempfile::NamedTempFile;

/// A program that increments the input
const PROGRAM_ADD_RET: &'static str = "
	add =>1
	ret 0
";

/// A program that returns the address of the stack base.
const STACK_BASE_PROGRAM: &'static str = "
	saddr u8 [0]
	echo =>end
	ret end
end:
";

/// Creates a temporary file with the `PROGRAM_ADD_RET` program.
fn create_add_ret_program_file() -> NamedTempFile
{
	create_temp_file(PROGRAM_ADD_RET.as_bytes())
}

/// Writes the stack base program to a temporary file as textual assembly.
fn stack_base_assembly_file() -> NamedTempFile
{
	create_temp_file(STACK_BASE_PROGRAM.as_bytes())
}

/// Tests can give input value relative to the built-in "entry" label.
#[test]
fn input_entry_offset()
{
	let assembly = Raw::assemble(once(PROGRAM_ADD_RET)).unwrap();

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
	let program_file = create_add_ret_program_file();
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
	assert!(events.contains("Instruction: (0x0) add =>1"));
	assert!(events.contains("Instruction: (0x2) ret 0"));
	assert_eq!(events.matches("STEP_END").count(), 2);
}

/// Tests that '--print-events' writes the simulation events to stdout when not
/// given a path
#[test]
fn print_events_to_stdout()
{
	let program_file = create_add_ret_program_file();

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

/// Runs a two-step program in machine mode with '--print-events' writing to a
/// file, checking that the events of every step (including the last) are in
/// the file when the program exits.
fn assert_machine_mode_print_events_to_file(buffered: bool)
{
	let program_file = create_add_ret_program_file();
	let events_file = tempfile::Builder::new().tempfile_in(TEMPORARY_DIR).unwrap();

	let mut cmd = cargo_bin_cmd!("scryer");
	cmd.arg(program_file.path())
		.arg("--target=assembly")
		.arg("--machine-mode")
		.arg("-i=1u8")
		.arg(format!(
			"--print-events={}",
			events_file.path().to_str().unwrap()
		));
	if buffered
	{
		cmd.arg("--print-events-buffered");
	}

	// Machine mode must still exit with the program's result and print nothing
	cmd.timeout(Duration::new(5, 0))
		.assert()
		.code(predicate::eq(2))
		.stdout(predicates::str::is_empty())
		.stderr(predicates::str::is_empty());

	// All steps must be in the file, including the final one
	let events = std::fs::read_to_string(events_file.path()).unwrap();
	assert!(
		events.contains("Instruction: (0x0) add =>1"),
		"First step missing from events file:\n{}",
		events
	);
	assert!(
		events.contains("Instruction: (0x2) ret 0"),
		"Last step missing from events file:\n{}",
		events
	);
	assert_eq!(events.matches("STEP_END").count(), 2);
}

/// Tests that in machine mode, unbuffered event printing writes the events of
/// the last step before exiting.
#[test]
fn machine_mode_print_events_to_file()
{
	assert_machine_mode_print_events_to_file(false);
}

/// Tests that in machine mode, buffered event printing writes all events
/// before exiting.
#[test]
fn machine_mode_print_events_buffered_to_file()
{
	assert_machine_mode_print_events_to_file(true);
}

/// Runs the given program file with the given target and '--stack-base' flag
/// (if any), checking that the returned stack address is as expected.
fn assert_stack_base(
	file: &std::path::Path,
	target: &str,
	stack_base: Option<&str>,
	expected: usize,
)
{
	let mut cmd = cargo_bin_cmd!("scryer");
	cmd.arg(file)
		.arg(format!("--target={}", target))
		.arg("-i=1u8");
	if let Some(stack_base) = stack_base
	{
		cmd.arg(format!("--stack-base={}", stack_base));
	}

	cmd.timeout(Duration::new(5, 0))
		.assert()
		.success()
		.stdout(
			predicate::str::is_match(format!("Returned Operands(.)*?\n{}u(32|64)", expected))
				.unwrap(),
		)
		.stderr(predicates::str::is_empty());
}

/// Tests that the stack base defaults to 1 MiB when '--stack-base' is not
/// given.
#[test]
fn stack_base_default()
{
	let file = stack_base_assembly_file();
	assert_stack_base(file.path(), "assembly", None, 1 << 20);
}

/// Tests that '--stack-base' accepts a decimal address.
#[test]
fn stack_base_decimal()
{
	let file = stack_base_assembly_file();
	assert_stack_base(file.path(), "assembly", Some("65536"), 65536);
}

/// Tests that '--stack-base' accepts a hexadecimal address.
#[test]
fn stack_base_hex()
{
	let file = stack_base_assembly_file();
	assert_stack_base(file.path(), "assembly", Some("0x20000"), 0x20000);
}

/// Tests that '--stack-base' accepts an ELF label, using its address.
#[test]
fn stack_base_elf32_label()
{
	test_stack_base_elf_label(false)
}

/// Tests that '--stack-base' accepts an ELF label, using its address.
#[test]
fn stack_base_elf64_label()
{
	test_stack_base_elf_label(true)
}

/// The address of the `my_stack` symbol in the ELF files created by
/// `stack_base_elf_file`.
const STACK_LABEL_ADDR: usize = 0x30000;

/// Writes the stack base program to a temporary ELF file that also contains a
/// `my_stack` symbol at `STACK_LABEL_ADDR`.
fn stack_base_elf_file(is_64_bit: bool) -> NamedTempFile
{
	let assembly = Raw::assemble(STACK_BASE_PROGRAM.split_whitespace()).unwrap();

	// Add a symbol to the ELF that names the stack location.
	// It is an absolute, untyped, global symbol, which is what a linker emits
	// for a symbol defined on its command line (e.g.
	// '--defsym=my_stack=0x30000')
	let mut elf = create_test_elf(assembly.as_slice(), 0, is_64_bit);
	let symtab = elf.sections.add();
	symtab.name = ".symtab".into();
	symtab.sh_type = SHT_SYMTAB;
	symtab.data = SectionData::Symbol;
	let strtab = elf.sections.add();
	strtab.name = ".strtab".into();
	strtab.sh_type = SHT_STRTAB;
	strtab.data = SectionData::String;
	let symbol = elf.symbols.add();
	symbol.name = "my_stack".into();
	symbol.section = None;
	symbol.st_shndx = SHN_ABS;
	symbol.set_st_info(STB_GLOBAL, STT_NOTYPE);
	symbol.st_value = STACK_LABEL_ADDR as u64;

	let mut elf_bytes = Vec::new();
	elf.write(&mut elf_bytes).unwrap();
	create_temp_file(elf_bytes.as_slice())
}

/// The target flag value for an ELF file of the given width.
fn elf_target(is_64_bit: bool) -> &'static str
{
	if is_64_bit
	{
		"scry64-unknown-none-elf"
	}
	else
	{
		"scry32-unknown-none-elf"
	}
}

fn test_stack_base_elf_label(is_64_bit: bool)
{
	let file = stack_base_elf_file(is_64_bit);
	assert_stack_base(
		file.path(),
		elf_target(is_64_bit),
		Some("my_stack"),
		STACK_LABEL_ADDR,
	);
}

/// Runs the given program file with a '--stack-base' label that does not
/// exist, checking that the simulator fails with an error naming the label
/// instead of running with the default stack base.
fn assert_stack_base_unknown_label(file: &std::path::Path, target: &str)
{
	let label = "no_such_label";

	let mut cmd = cargo_bin_cmd!("scryer");
	cmd.arg(file)
		.arg(format!("--target={}", target))
		.arg("-i=1u8")
		.arg(format!("--stack-base={}", label));

	cmd.timeout(Duration::new(5, 0))
		.assert()
		.failure()
		// The program must not have been simulated
		.stdout(predicate::str::contains("Returned Operands").not())
		// The error must name the missing label
		.stderr(predicate::str::contains(
			"Missing stack base label: no_such_label",
		));
}

/// Tests that '--stack-base' given a label missing from the ELF is an error.
#[test]
fn stack_base_elf_unknown_label()
{
	let file = stack_base_elf_file(false);
	assert_stack_base_unknown_label(file.path(), elf_target(false));
}

/// Writes the `PROGRAM_ADD_RET` program to a temporary ELF file of the given
/// width.
fn add_ret_elf_file(is_64_bit: bool) -> NamedTempFile
{
	let assembly = Raw::assemble(once(PROGRAM_ADD_RET)).unwrap();
	let elf = create_test_elf(assembly.as_slice(), 0, is_64_bit);
	let mut elf_bytes = Vec::new();
	elf.write(&mut elf_bytes).unwrap();
	create_temp_file(elf_bytes.as_slice())
}

/// Runs the given `PROGRAM_ADD_RET` file in machine mode with the given
/// '--target' flag (if any), checking that the program was simulated correctly
/// (input 1 increments to 2).
fn assert_add_ret_runs(file: &std::path::Path, target: Option<&str>)
{
	let mut cmd = cargo_bin_cmd!("scryer");
	cmd.arg(file).arg("--machine-mode").arg("-i=1u8");
	if let Some(target) = target
	{
		cmd.arg(format!("--target={}", target));
	}

	cmd.timeout(Duration::new(5, 0))
		.assert()
		.code(predicate::eq(2))
		.stdout(predicates::str::is_empty())
		.stderr(predicates::str::is_empty());
}

/// Tests that '--target=auto' detects a 32-bit ELF file.
#[test]
fn target_auto_elf32()
{
	let file = add_ret_elf_file(false);
	assert_add_ret_runs(file.path(), Some("auto"));
}

/// Tests that '--target=auto' detects a 64-bit ELF file.
#[test]
fn target_auto_elf64()
{
	let file = add_ret_elf_file(true);
	assert_add_ret_runs(file.path(), Some("auto"));
}

/// Tests that '--target=auto' detects a textual assembly file.
#[test]
fn target_auto_assembly()
{
	let file = create_add_ret_program_file();
	assert_add_ret_runs(file.path(), Some("auto"));
}

/// Tests that '--target=auto' never detects a raw binary file.
#[test]
fn target_auto_raw()
{
	let assembly = Raw::assemble(once(PROGRAM_ADD_RET)).unwrap();
	// Raw binaries can only be told apart from textual assembly if they are
	// not valid text, so make sure this test's program is unambiguous.
	assert!(
		std::str::from_utf8(assembly.as_slice()).is_err(),
		"Test program must not encode to valid UTF-8: {:?}",
		assembly
	);
	let file = create_temp_file(assembly.as_slice());
	let mut cmd = cargo_bin_cmd!("scryer");
	cmd.arg(file.path())
		.arg("--machine-mode")
		.arg("-i=1u8")
		.arg("--target=auto");

	cmd.timeout(Duration::new(5, 0))
		.assert()
		.code(predicate::eq(123))
		.stderr(predicates::str::contains("Unable to determine target"));
}

/// Tests that omitting '--target' defaults to autodetection.
#[test]
fn target_default_is_auto()
{
	let file = add_ret_elf_file(false);
	assert_add_ret_runs(file.path(), None);
}

/// Tests that '--target=auto' rejects an ELF file for another architecture
#[test]
fn target_auto_elf_wrong_machine()
{
	let assembly = Raw::assemble(once(PROGRAM_ADD_RET)).unwrap();
	let mut elf = create_test_elf(assembly.as_slice(), 0, false);
	elf.header.e_machine = EM_X86_64;
	let mut elf_bytes = Vec::new();
	elf.write(&mut elf_bytes).unwrap();
	let file = create_temp_file(elf_bytes.as_slice());

	let mut cmd = cargo_bin_cmd!("scryer");
	cmd.arg(file.path())
		.arg("--machine-mode")
		.arg("-i=1u8")
		.arg("--target=auto");

	cmd.timeout(Duration::new(5, 0))
		.assert()
		.failure()
		.stdout(predicates::str::is_empty())
		.stderr(predicates::str::contains("Unable to determine target"));
}
