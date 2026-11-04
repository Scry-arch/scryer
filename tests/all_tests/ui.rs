use crate::{all_tests::create_test_elf, TEMPORARY_DIR};
use assert_cmd::cargo_bin_cmd;
use object::{
	build::elf::SectionData,
	elf::{SHN_ABS, SHT_STRTAB, SHT_SYMTAB, STB_GLOBAL, STT_NOTYPE},
};
use predicates::prelude::{predicate, PredicateBooleanExt};
use scry_asm::{Assemble, Raw};
use std::{io::Write, iter::once, time::Duration};

/// Tests can give input value relative to the built-in "entry" label.
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

/// A program that returns the address of the stack base.
const STACK_BASE_PROGRAM: [&str; 4] = ["saddr u8 [0]", "echo =>end", "ret end", "end:"];

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

/// Writes the stack base program to a temporary file as textual assembly.
fn stack_base_assembly_file() -> tempfile::NamedTempFile
{
	std::fs::create_dir_all(TEMPORARY_DIR).unwrap();
	let file = tempfile::Builder::new().tempfile_in(TEMPORARY_DIR).unwrap();
	file.as_file()
		.write_all(STACK_BASE_PROGRAM.join("\n").as_bytes())
		.unwrap();
	file
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

fn test_stack_base_elf_label(is_64_bit: bool)
{
	let stack_label_addr = 0x30000;
	let assembly = Raw::assemble(STACK_BASE_PROGRAM.iter().cloned()).unwrap();

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
	symbol.st_value = stack_label_addr as u64;

	let mut elf_bytes = Vec::new();
	elf.write(&mut elf_bytes).unwrap();
	std::fs::create_dir_all(TEMPORARY_DIR).unwrap();
	let file = tempfile::Builder::new().tempfile_in(TEMPORARY_DIR).unwrap();
	file.as_file().write_all(elf_bytes.as_slice()).unwrap();

	assert_stack_base(
		file.path(),
		if is_64_bit
		{
			"scry64-unknown-none-elf"
		}
		else
		{
			"scry32-unknown-none-elf"
		},
		Some("my_stack"),
		stack_label_addr,
	);
}
