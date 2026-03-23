use crate::{all_tests::create_test_elf, TEMPORARY_DIR};
use assert_cmd::cargo_bin_cmd;
use predicates::prelude::predicate;
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
		let elf = create_test_elf(assembly.as_slice(), entry_addr);
		let mut elf_bytes = Vec::new();
		elf.write(&mut elf_bytes).unwrap();
		std::fs::create_dir_all(TEMPORARY_DIR).unwrap();
		let file = tempfile::Builder::new().tempfile_in(TEMPORARY_DIR).unwrap();
		file.as_file().write_all(elf_bytes.as_slice()).unwrap();

		// Run on the file with relative input
		let mut cmd = cargo_bin_cmd!("scryer");
		cmd.arg(file.path());
		cmd.arg("--machine-mode");
		cmd.arg("--target=scry-unknown-none-elf32");
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
