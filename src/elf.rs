use object::{
	elf::{EM_SCRY, PT_LOAD},
	read::elf::{ElfFile, FileHeader, ProgramHeader},
	Endianness, Object, ObjectSegment, ObjectSymbol, SymbolKind,
};
use scry_sim::{BlockedMemory, Memory};
use std::collections::HashMap;

/// Extracts the contents of an ELF file (both 32- and 64-bit), returning the
/// entry addres, the memory, and the address width in powers of 2, and the
/// symbol table.
pub fn extract_elf<'a, E: FileHeader>(
	elf: ElfFile<'a, E, &'a [u8]>,
) -> (usize, BlockedMemory, u8, HashMap<String, usize>)
{
	let mut mem = BlockedMemory::empty();
	for segment in elf.segments()
	{
		if segment.elf_program_header().p_type(elf.endian()) == PT_LOAD
		{
			if let Ok(data) = segment.data()
			{
				let addr = segment.address() as usize;
				let size = segment.size() as usize;

				mem.add_block_zeroed(addr, size);
				data.iter()
					.enumerate()
					.for_each(|(i, b)| mem.write_raw(addr + i, *b).unwrap())
			}
		}
	}

	let symbols = elf
		.symbols()
		.filter(|sym| {
			!sym.is_undefined()
				&& matches!(
					sym.kind(),
					SymbolKind::Text | SymbolKind::Data | SymbolKind::Unknown
				)
		})
		.filter_map(|sym| {
			let name = sym.name().ok()?;
			if name.is_empty()
			{
				return None;
			}
			Some((name.to_string(), sym.address() as usize))
		})
		.collect();

	(
		elf.entry() as usize,
		mem,
		if elf.is_64() { 3 } else { 2 },
		symbols,
	)
}

/// Extracts the ELF file into its entry point, memory, the address width in
/// powers of 2, and the stack base
pub fn extract_elf_and_stack_base<'a, E: FileHeader<Endian = Endianness>>(
	elf: ElfFile<'a, E, &'a [u8]>,
	stack_base_arg: &String,
) -> Result<(usize, BlockedMemory, u8, Option<usize>), ()>
{
	let mut stack_base_result = None;
	if elf.elf_header().e_machine(Endianness::Little) != EM_SCRY
	{
		return Err(());
	}
	let (e, m, a, syms) = extract_elf(elf);

	if !stack_base_arg.is_empty()
		&& stack_base_arg.parse::<usize>().is_err()
		&& !stack_base_arg.starts_with("0x")
	{
		if let Some(addr) = syms.get(stack_base_arg)
		{
			stack_base_result = Some(*addr);
		}
		else
		{
			return Err(()); // Label not found
		}
	}

	Ok((e, m, a, stack_base_result))
}
