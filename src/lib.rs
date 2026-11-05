#![cfg_attr(feature = "fail-on-warnings", deny(warnings))]

use crate::events::StepTracker;
use clap::Parser;
use num_bigint::{BigInt, BigUint, Sign};
use object::{
	elf::PT_LOAD,
	read::elf::{ElfFile, FileHeader, ProgramHeader},
	Object, ObjectSegment, ObjectSymbol, SymbolKind,
};
use regex::Regex;
use scry_asm::{Assemble, Disassemble};
use scry_sim::{
	Block, BlockedMemory, CallFrameState, Event, ExecError, ExecState, Executor, IdEvent, Memory,
	OperandEvent, OperandList, Scalar, StackFrame, Value, ValueType,
};
use std::{
	collections::HashMap,
	io::{BufWriter, Write},
	time::Instant,
};

mod events;

#[derive(clap::ValueEnum, Clone, Eq, PartialEq)]
enum TimeoutType
{
	Instructions,
	Seconds,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum Target
{
	/// Tries to detect the target automatically
	Auto,
	/// Raw binary file containing only encoded instructions
	Raw,
	/// File containing textual assembly
	Assembly,
	/// 32-bit ELF file
	Scry32UnknownNoneElf,
	/// 64-bit ELF file
	Scry64UnknownNoneElf,
}

/// Command-line arguments
#[derive(Parser)]
pub struct Cli
{
	/// The path to the file to execute
	path: std::path::PathBuf,

	/// For when the simulator needs to emulate the program exactly.
	/// In this mode, the simulator's outputs (exit code, stdout, stderr) come
	/// from the simulated program.
	#[clap(short, long)]
	machine_mode: bool,

	/// The target triple of the input file
	#[clap(long, value_name = "TRIPLE")]
	#[arg(value_enum, default_value_t = Target::Auto)]
	target: Target,

	/// Input operand to the first instruction.
	/// Can be given multiple times for multiple input operands.
	/// Must give value and its type. E.g. '1u8' is an unsigned byte of value 1.
	/// Can give relative to a known label. E.g. 'entry+1u8' is an unsigned byte
	/// of value one higher than the 'entry' label.
	#[clap(short, long)]
	input: Vec<String>,

	/// Stop the simulation early.
	#[clap(long)]
	#[arg(default_value_t = 0)]
	timeout: usize,

	/// What counter should be used to timeout.
	#[clap(long)]
	#[arg(value_enum, default_value_t = TimeoutType::Seconds)]
	timeout_type: TimeoutType,

	/// Prints simulation events either to stdout or a file
	#[clap(long, value_name = "FILE", num_args = 0..=1, require_equals = true)]
	print_events: Option<Option<std::path::PathBuf>>,

	/// Event will be buffered and only printing after the simulation terminates
	#[clap(long)]
	#[arg(default_value_t = false)]
	print_events_buffered: bool,

	/// Where the initial stack base should be located.
	///
	/// If given an integer (decimal or hexadecimal), the stack base starts at
	/// that address. If given a string label, the corresponding address will be
	/// used.
	#[clap(long, value_name = "ADDRESS")]
	#[arg(default_value_t = String::new())]
	stack_base: String,

	/// Initial size of the stack buffer.
	#[clap(long, value_name = "BYTES")]
	#[arg(default_value_t = 1 << 12)]
	stack_buffer: usize,
}

fn parse_input(input: &String, labels: HashMap<&str, usize>) -> Value
{
	let re = Regex::new(r"^((.+)\+)?(-?\d+)(u|i)(\d+)$").unwrap();
	let caps = re.captures_iter(input.as_str()).next().unwrap();

	let bit_size: u8 = caps[5].parse().unwrap();
	let byte_size = bit_size / 8;
	let byte_size_pow_2 = byte_size.ilog2() as u8;
	let signed = &caps[4] == "i";
	let value = caps[3].as_bytes();
	let relative_label = caps.get(2);
	let add_to_value = || {
		if let Some(label) = relative_label
		{
			if let Some(addr) = labels.get(label.as_str())
			{
				return *addr;
			}
		}
		0
	};

	let typ = if signed
	{
		ValueType::Int(byte_size_pow_2)
	}
	else
	{
		ValueType::Uint(byte_size_pow_2)
	};
	let (sign, mut value_bytes) = if signed
	{
		let mut value = BigInt::parse_bytes(&value, 10).unwrap();
		value += add_to_value();
		(value.sign(), value.to_signed_bytes_le())
	}
	else
	{
		let mut value = BigUint::parse_bytes(value, 10).unwrap();
		value += add_to_value();
		(Sign::Plus, value.to_bytes_le())
	};
	// Ensure the number of bytes fits the type
	value_bytes.resize(
		byte_size as usize,
		if sign == Sign::Minus { u8::MAX } else { 0 },
	);
	assert_eq!(value_bytes.len(), byte_size as usize);

	Value::singleton_typed(typ, Scalar::Val(value_bytes.into_boxed_slice()))
}

fn value_to_string(val: &Value) -> String
{
	let (mut val, typ, size_pow_2): (String, char, u8) = match val.value_type()
	{
		ValueType::Uint(x) =>
		{
			let int = BigUint::from_bytes_le(val.iter().next().unwrap().bytes().unwrap());
			(int.to_string(), 'u', 2u8.pow(x as u32) * 8)
		},
		ValueType::Int(x) =>
		{
			let int = BigInt::from_signed_bytes_le(val.iter().next().unwrap().bytes().unwrap());
			(int.to_string(), 'i', 2u8.pow(x as u32) * 8)
		},
	};
	val.push(typ);
	val.push_str(&*size_pow_2.to_string());
	val
}

fn print_events(tracker: &StepTracker, out: &mut impl Write)
{
	for step_events in tracker.all_events()
	{
		for event in step_events
		{
			match event
			{
				Event::Instruction(addr, instr) =>
				{
					write!(
						out,
						"Instruction: ({:#x}) {}",
						addr,
						scry_asm::Raw::disassemble(instr.encode().to_le_bytes().into_iter())
							.unwrap()
					)
				},
				Event::Id(id, id_typ) =>
				{
					match id_typ
					{
						IdEvent::Operand(OperandEvent::Produce(t)) =>
						{
							write!(out, "Operand: Produce({},", id).unwrap();
							if t.is_signed_int()
							{
								write!(out, "i").unwrap();
							}
							else if t.is_unsigned_int()
							{
								write!(out, "u").unwrap();
							}
							else
							{
								todo!()
							}
							writeln!(out, "{})", t.size() * 8)
						},
						IdEvent::Operand(p) => writeln!(out, "Operand: {:?}({})", p, id),
						IdEvent::ControlIssue(i) => writeln!(out, "Issue: {:?}({})", i, id),
						IdEvent::ControlTrigger(t) => writeln!(out, "Trigger: {:?}({})", t, id),
						_ => writeln!(out, "IdEvent({}): {:?}", id, id_typ),
					}
				},
				Event::Load(addr, idx, bytes) =>
				{
					if let Some(idx) = idx
					{
						writeln!(out, "LoadStack({:#x},{}): {}", addr, idx, bytes)
					}
					else
					{
						writeln!(out, "Load({:#x}): {}", addr, bytes)
					}
				},
				Event::Store(addr, idx, bytes) =>
				{
					if let Some(idx) = idx
					{
						writeln!(out, "StoreStack({:#x},{}): {}", addr, idx, bytes)
					}
					else
					{
						writeln!(out, "Store({:#x}): {}", addr, bytes)
					}
				},
				Event::Stack(section, addr) =>
				{
					writeln!(out, "Stack: {:?}({:#x})", section, addr)
				},
				_ => writeln!(out, "Event: {:?}", event),
			}
			.unwrap();
		}
		writeln!(out, "STEP_END").unwrap();
	}
}

fn print_events_final(tracker: &StepTracker, out: &mut Option<Box<dyn Write>>, buffering: bool)
{
	if let Some(out) = out
	{
		if buffering
		{
			writeln!(out, "\nvvvvvvvvvv  Simulation Events  vvvvvvvvvv").unwrap();
		}
		print_events(&tracker, out);
		if buffering
		{
			writeln!(out, "\n^^^^^^^^^^  Simulation Events  ^^^^^^^^^^").unwrap();
		}
	}
}

/// Extracts the contents of an ELF file (both 32- and 64-bit), returning the
/// entry addres, the memory, and the address width in powers of 2, and the
/// symbol table.
fn extract_elf<'a, E: FileHeader>(
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

/// Extracts the ELF file and overwrites the give stack base if necessary
fn extract_elf_and_stack_base<'a, E: FileHeader>(
	elf: ElfFile<'a, E, &'a [u8]>,
	stack_base_arg: &String,
	stack_base: &mut usize,
) -> Result<(usize, BlockedMemory, u8), ()>
{
	let (e, m, a, syms) = extract_elf(elf);

	if !stack_base_arg.is_empty()
		&& stack_base_arg.parse::<usize>().is_err()
		&& !stack_base_arg.starts_with("0x")
	{
		if let Some(addr) = syms.get(stack_base_arg)
		{
			*stack_base = *addr;
		}
		else
		{
			return Err(()); // Label not found
		}
	}

	Ok((e, m, a))
}

pub fn run(args: Cli, mut stdout: impl Write, _stderr: impl Write) -> i32
{
	// Read program
	let contents = std::fs::read(args.path).unwrap();

	// Prepare event output
	let mut event_print_out = args.print_events.map(|p| -> Box<dyn Write> {
		match p
		{
			None => Box::new(std::io::stdout()),
			Some(path) => Box::new(BufWriter::new(std::fs::File::create(path).unwrap())),
		}
	});

	// Default stack base
	let mut stack_base = 1 << 20;

	// Check for absolute stack base
	if args.stack_base.starts_with("0x")
	{
		if let Ok(addr) = usize::from_str_radix(args.stack_base.trim_start_matches("0x"), 16)
		{
			stack_base = addr;
		}
	}
	else if let Ok(addr) = args.stack_base.parse::<usize>()
	{
		stack_base = addr;
	}

	let (entry, mut memory, address_space) = match args.target
	{
		Target::Auto =>
		{
			unimplemented!()
		},
		Target::Raw => (0, BlockedMemory::new(contents.into_iter(), 0), 2),
		Target::Assembly =>
		{
			// File is in textual assembly, assemble it
			let program = scry_asm::Raw::assemble(std::iter::once(
				String::from_utf8(contents).unwrap().as_str(),
			))
			.unwrap();

			(0, BlockedMemory::new(program.into_iter(), 0), 2)
		},
		Target::Scry32UnknownNoneElf =>
		{
			if let object::File::Elf32(elf) = object::File::parse(&*contents).unwrap()
			{
				extract_elf_and_stack_base(elf, &args.stack_base, &mut stack_base)
					.expect(format!("Missing stack base label: {}", &args.stack_base).as_str())
			}
			else
			{
				unimplemented!()
			}
		},
		Target::Scry64UnknownNoneElf =>
		{
			if let object::File::Elf64(elf) = object::File::parse(&*contents).unwrap()
			{
				extract_elf_and_stack_base(elf, &args.stack_base, &mut stack_base)
					.expect(format!("Missing stack base label: {}", &args.stack_base).as_str())
			}
			else
			{
				unimplemented!()
			}
		},
	};

	// Ready inputs
	let mut op_queue = HashMap::new();
	if !args.input.is_empty()
	{
		let mut ops: Vec<_> = args
			.input
			.iter()
			.map(|s| {
				let labels = [("entry", entry)].into_iter().collect();
				parse_input(s, labels)
			})
			.collect();
		op_queue.insert(0, OperandList::new(ops.remove(0), ops));
	}

	let base_stack = StackFrame {
		block: Block {
			address: stack_base,
			size: 0,
		},
		private_size: 0,
	};
	let original_state = ExecState {
		addr_space: address_space,
		address: entry,
		frame: CallFrameState {
			ret_addr: 0,
			branches: HashMap::new(),
			op_queue,
			stack: base_stack.clone(),
		},
		frame_stack: vec![CallFrameState {
			ret_addr: 0,
			branches: HashMap::new(),
			op_queue: HashMap::new(),
			stack: base_stack,
		}],
		stack_buffer: args.stack_buffer,
		more_depth: 1, // Total depth = 4
	};
	let mut events = StepTracker::new();

	// Add stack memery block
	memory.add_block((0..args.stack_buffer).map(|_| 0), stack_base);

	let mut res =
		Executor::<BlockedMemory, _>::from_state(&original_state, &mut memory, &mut events)
			.step(&mut events);

	let start_time = Instant::now();
	let mut step_count = 1;
	while res.is_ok()
	{
		let exec = res.unwrap();
		let state = exec.state();

		if state.frame_stack.len() == 0
		{
			// Done
			if let Some(returned_values) = state.frame.op_queue.get(&0)
			{
				print_events_final(&events, &mut event_print_out, args.print_events_buffered);
				if args.machine_mode
				{
					// Return the integer value of the first return operand or
					// 123 if unavailable
					return returned_values
						.iter()
						.next()
						.unwrap()
						.get_first()
						.bytes()
						.map_or(123, |b| b[0] as i32);
				}
				else
				{
					// Pretty print the returned operands
					writeln!(stdout, "----------  Returned Operands  ----------").unwrap();
					for val in returned_values.iter()
					{
						let val_str = value_to_string(val);
						write!(stdout, "{}, ", val_str).unwrap();
					}

					return 0;
				}
			}
			// Failure
			res = Err(ExecError::Err("No operands returned by simulation.".into()));
			continue;
		}

		if args.timeout > 0
			&& ((args.timeout_type == TimeoutType::Instructions
				&& step_count == args.timeout) // We assume one instruction per step
				| (args.timeout_type == TimeoutType::Seconds
					&& start_time.elapsed().as_secs() > args.timeout as u64))
		{
			print_events_final(&events, &mut event_print_out, args.print_events_buffered);
			if !args.machine_mode
			{
				writeln!(stdout, "----------  Timeout  ----------").unwrap();
			}
			return 123;
		}

		if let Some(out) = &mut event_print_out
		{
			if !args.print_events_buffered
			{
				print_events(&events, out);
				events.clear_events()
			}
		}

		events.new_step();
		res = exec.step(&mut events);
		step_count += 1;
	}
	// Implicit failure
	match res
	{
		Err(err) =>
		{
			print_events_final(&events, &mut event_print_out, args.print_events_buffered);
			if !args.machine_mode
			{
				writeln!(stdout, "----------  Error  ----------").unwrap();
				writeln!(stdout, "{:?}", err).unwrap();
			}
		},
		Ok(_) => unreachable!(),
	}
	123
}
