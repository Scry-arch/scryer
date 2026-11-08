use crate::TEMPORARY_DIR;
use object::{
	build::elf::{Builder, SectionData},
	elf::{
		ProgramFlags, ELFOSABI_NONE, EM_SCRY, ET_EXEC, PF_R, PF_W, PF_X, PT_PHDR, SHF_ALLOC,
		SHF_EXECINSTR, SHF_WRITE, SHT_NOBITS, SHT_PROGBITS,
	},
	Endianness,
};
use std::io::Write;
use tempfile::NamedTempFile;

mod programs;
mod timeout;
mod ui;

/// A writable data segment to add to a test ELF.
///
/// The segment contains a `.data` section with the given initialized bytes,
/// immediately followed (8-byte aligned) by a `.bss` section of the given size,
/// which takes up no space in the file.
struct TestDataSegment<'a>
{
	/// The address the segment (and therefore `.data`) is mapped to.
	addr: usize,
	/// The contents of the `.data` section.
	data: &'a [u8],
	/// The size of the `.bss` section in bytes.
	bss_size: usize,
}

/// Creates an ELF file for testing containing the given code (.text section).
///
/// The ELF maps the .text section to the given address.
fn create_test_elf(code: &[u8], entry_addr: usize, is_64_bit: bool) -> Builder<'_>
{
	create_test_elf_with_data(code, entry_addr, is_64_bit, None)
}

/// Creates an ELF file for testing containing the given code (.text section)
/// and, optionally, a writable data segment (.data and .bss sections).
///
/// The ELF maps the .text section to the given address.
fn create_test_elf_with_data<'a>(
	code: &'a [u8],
	entry_addr: usize,
	is_64_bit: bool,
	data_segment: Option<TestDataSegment<'a>>,
) -> Builder<'a>
{
	let mut elf = Builder::new(Endianness::Little, is_64_bit);
	elf.header.os_abi = ELFOSABI_NONE;
	elf.header.e_type = ET_EXEC;
	elf.header.e_machine = EM_SCRY;
	elf.header.e_phoff = elf.file_header_size() as u64;

	// Create segments
	let phdr_seg = elf.segments.add().id();
	let load_phdr_seg = elf.segments.add_load_segment(ProgramFlags(0), 4).id();
	let program_seg = elf.segments.add_load_segment(ProgramFlags(0), 4).id();
	// All segments must be created before the program header size is
	// calculated
	let data_seg = data_segment
		.as_ref()
		.map(|_| elf.segments.add_load_segment(ProgramFlags(0), 4).id());
	let phdr_size = elf.program_headers_size();

	// Config PHDR segment
	elf.segments.get_mut(phdr_seg).p_type = PT_PHDR;
	elf.segments.get_mut(phdr_seg).p_offset = elf.header.e_phoff;
	elf.segments.get_mut(phdr_seg).p_flags = PF_R;
	elf.segments.get_mut(phdr_seg).p_align = 4;
	elf.segments.get_mut(phdr_seg).p_filesz = phdr_size;
	elf.segments.get_mut(phdr_seg).p_memsz = phdr_size;

	// Config segment to contain sections: .text
	elf.segments.get_mut(program_seg).p_offset = elf.header.e_phoff + phdr_size;
	elf.segments.get_mut(program_seg).p_vaddr = entry_addr as u64;
	elf.segments.get_mut(program_seg).p_paddr = elf.segments.get(program_seg).p_vaddr;
	elf.segments.get_mut(program_seg).p_flags = PF_R | PF_X;

	// Create .text section
	let text_section = elf.sections.add();
	text_section.name = ".text".into();
	text_section.sh_type = SHT_PROGBITS;
	text_section.sh_addralign = 2;
	text_section.sh_size = code.len() as u64;
	text_section.sh_flags = SHF_ALLOC | SHF_EXECINSTR;
	text_section.data = SectionData::Data(code.into());

	elf.segments
		.get_mut(program_seg)
		.append_section(text_section);

	// Set program entry point to .text section start
	elf.header.e_entry = elf.segments.get(program_seg).p_paddr;

	elf.segments.get_mut(phdr_seg).p_vaddr =
		elf.segments.get(program_seg).p_vaddr + elf.segments.get(program_seg).p_filesz;
	elf.segments.get_mut(phdr_seg).p_paddr = elf.segments.get(phdr_seg).p_vaddr;

	// Config segment to contain sections: PHDR
	// Must be mapped to after the .text section
	elf.segments.get_mut(load_phdr_seg).p_offset = elf.segments.get(phdr_seg).p_offset;
	elf.segments.get_mut(load_phdr_seg).p_flags = PF_R;

	// Add PHDR to the load segment through a fake section
	let phdr_sec_id = elf.sections.add().id();
	elf.sections.get_mut(phdr_sec_id).sh_size = phdr_size;
	elf.sections.get_mut(phdr_sec_id).sh_offset = elf.segments.get(load_phdr_seg).p_offset;
	elf.segments
		.get_mut(load_phdr_seg)
		.append_section_range(elf.sections.get_mut(phdr_sec_id));
	// Set addresses to after .text segment
	elf.segments.get_mut(load_phdr_seg).p_vaddr = elf.segments.get(phdr_seg).p_vaddr;
	elf.segments.get_mut(load_phdr_seg).p_paddr = elf.segments.get(phdr_seg).p_vaddr;
	elf.sections.get_mut(phdr_sec_id).delete = true;

	// Config segment to contain sections: .data, .bss
	if let (Some(data_seg), Some(data_segment)) = (data_seg, data_segment)
	{
		// In the file, the data comes right after .text
		let data_offset = elf.header.e_phoff + phdr_size + code.len() as u64;
		let segment = elf.segments.get_mut(data_seg);
		segment.p_offset = data_offset;
		segment.p_vaddr = data_segment.addr as u64;
		segment.p_paddr = data_segment.addr as u64;
		segment.p_flags = PF_R | PF_W;

		// Create .data section
		let data_section = elf.sections.add();
		data_section.name = ".data".into();
		data_section.sh_type = SHT_PROGBITS;
		data_section.sh_addralign = 8;
		data_section.sh_size = data_segment.data.len() as u64;
		data_section.sh_flags = SHF_ALLOC | SHF_WRITE;
		data_section.data = SectionData::Data(data_segment.data.into());
		elf.segments.get_mut(data_seg).append_section(data_section);

		// Create .bss section (takes no space in the file, only in memory)
		let bss_section = elf.sections.add();
		bss_section.name = ".bss".into();
		bss_section.sh_type = SHT_NOBITS;
		bss_section.sh_addralign = 8;
		bss_section.sh_size = data_segment.bss_size as u64;
		bss_section.sh_flags = SHF_ALLOC | SHF_WRITE;
		bss_section.data = SectionData::UninitializedData(data_segment.bss_size as u64);
		elf.segments.get_mut(data_seg).append_section(bss_section);
	}

	// Create section header string table section
	let shstrtab_section = elf.sections.add();
	shstrtab_section.name = ".shstrtab".into();
	shstrtab_section.data = SectionData::SectionString;

	elf
}

/// Creates a temporary file containing the given bytes
fn create_temp_file(content: &[u8]) -> NamedTempFile
{
	std::fs::create_dir_all(TEMPORARY_DIR).unwrap();
	let program_file = tempfile::Builder::new().tempfile_in(TEMPORARY_DIR).unwrap();
	program_file.as_file().write_all(content).unwrap();
	program_file
}
