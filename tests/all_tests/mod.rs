use object::{
	build::elf::{Builder, SectionData},
	elf::{
		ELFOSABI_NONE, EM_SCRY, ET_EXEC, PF_R, PF_X, PT_PHDR, SHF_ALLOC, SHF_EXECINSTR,
		SHT_PROGBITS,
	},
	Endianness,
};

mod programs;
mod timeout;
mod ui;

/// Creates an ELF file for testing containing the given code (.text section).
///
/// The ELF maps the .text section to the given address.
fn create_test_elf(code: &[u8], entry_addr: usize) -> Builder<'_>
{
	let mut elf = Builder::new(Endianness::Little, false);
	elf.header.os_abi = ELFOSABI_NONE;
	elf.header.e_type = ET_EXEC;
	elf.header.e_machine = EM_SCRY;
	elf.header.e_phoff = elf.file_header_size() as u64;

	// Create segments
	let phdr_seg = elf.segments.add().id();
	let load_phdr_seg = elf.segments.add_load_segment(0, 4).id();
	let program_seg = elf.segments.add_load_segment(0, 4).id();
	let phdr_size = elf.program_headers_size() as u64;

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
	text_section.sh_flags = (SHF_ALLOC | SHF_EXECINSTR) as u64;
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

	// Create section header string table section
	let shstrtab_section = elf.sections.add();
	shstrtab_section.name = ".shstrtab".into();
	shstrtab_section.data = SectionData::SectionString;

	elf
}
